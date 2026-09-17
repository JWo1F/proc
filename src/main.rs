//! A process manager that runs commands defined in a Procfile.
//!
//! Each process is spawned inside its own PTY, and output lines are
//! multiplexed to stdout with a colored name prefix.
//!
//! Also supports pipe mode: `my_command | procfile` reads stdin and
//! displays it through the same stdout/web consumers.

use crate::core::manager::{OnExit, ProcessManager};
use crate::core::procfile::ProcessSpec;
use clap::{Parser, Subcommand};
use std::fs;
use std::io::IsTerminal;
use std::process::ExitCode;
use tokio::sync::{broadcast, mpsc, watch};

mod core;
mod input;
mod pipe;
mod stdout;
mod tui;
#[cfg(feature = "web")]
mod web;

const DEFAULT_CONFIG: &str = "Procfile";

#[derive(Parser, Debug)]
struct RunOptions {
  /// Process names to run (runs all if omitted)
  names: Vec<String>,

  /// Path to the Procfile
  #[arg(short, long, default_value = DEFAULT_CONFIG)]
  config: String,

  /// Inline process definition: "name: command" (repeatable, skips file if no -c given)
  #[arg(short = 'r', long = "run")]
  run: Vec<String>,

  /// Behavior when a process exits [default: stop; ignore in interactive mode]
  #[arg(value_enum, long)]
  on_exit: Option<OnExit>,

  /// Bring an `optional` process into the run (repeatable)
  #[arg(short = 'e', long = "enable")]
  enable: Vec<String>,

  /// Bring every `optional` process into the run
  #[arg(short = 'E', long = "enable-all")]
  enable_all: bool,

  /// Prefix each line with a timestamp
  #[arg(short = 'T', long)]
  timestamps: bool,

  /// Hide process names, show only colored bars
  #[arg(short = 's', long)]
  compact: bool,

  /// Suppress log output to stdout (use with -w for web-only)
  #[arg(short = 'q', long)]
  silent: bool,

  /// Hide system messages (Spawned, Stopped, etc.)
  #[arg(long)]
  no_system: bool,

  /// Disable interactive mode (on by default in a terminal)
  #[arg(short = 'I', long)]
  no_interactive: bool,

  /// Start SSE server (optional port, default: derived from folder name)
  #[cfg(feature = "web")]
  #[arg(short = 'w', long, num_args = 0..=1, default_missing_value = "0")]
  web: Option<u16>,
}

impl RunOptions {
  /// Resolve the on-exit policy, which defaults differently by mode.
  ///
  /// Headless, `Stop` is right: one process dying usually invalidates the whole
  /// run. Interactively the session outlives any individual process — the
  /// modal is still there to inspect what happened and start it again — so a
  /// process ending on its own must not tear the session down.
  fn effective_on_exit(&self, interactive: bool) -> OnExit {
    self.on_exit.unwrap_or(if interactive {
      OnExit::Ignore
    } else {
      OnExit::Stop
    })
  }
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
#[command(after_help = "\
Procfile format:
  <name>: <command>
  <name>(<flags>): <command>

  Lines starting with '#' are comments.

Process flags (comma separated, apply to that one process only):
  optional        keep it out of the run until it is asked for by name,
                  with -e/--enable, or from the interactive menu
  once            when it exits, leave it down — no restart, and the
                  rest of the run keeps going
  restart         always respawn it when it exits
  stop            stop the whole run when it exits
  delay=<dur>     hold its automatic start (500ms, 2s, 1m; bare = seconds)
  retries=<n>     give up after n consecutive automatic restarts
  muted           keep its output out of the terminal (the web feed
                  still receives it)
  allow-failure   a non-zero exit from it does not fail the run

  Only one of once/restart/stop per process — they are the same setting.

Examples:
  procfile
  procfile web worker
  procfile -e seed
  procfile -r 'extra: sidekiq'
  procfile -r 'migrate(once): rake db:migrate'
  procfile check
  procfile list

Procfile example:
  db: postgres -D ./tmp/db
  web(delay=2s): bin/rails server
  worker(restart, retries=5): bundle exec sidekiq
  logs(muted): tail -f log/development.log
  seed(optional, once, allow-failure): bin/rails db:seed")]
struct Args {
  #[command(subcommand)]
  command: Option<Command>,

  #[command(flatten)]
  run: RunOptions,
}

#[derive(Subcommand, Debug)]
enum Command {
  /// Start processes (default — same as running without a subcommand)
  Start(RunOptions),

  /// Validate Procfile syntax
  Check {
    /// Path to the Procfile
    #[arg(short, long, default_value = DEFAULT_CONFIG)]
    config: String,
  },

  /// List processes defined in a Procfile
  List {
    /// Path to the Procfile
    #[arg(short, long, default_value = DEFAULT_CONFIG)]
    config: String,
  },
}

/// Parse --run "name: command" into process definitions, reusing Procfile
/// syntax so inline entries accept the same flags a file line does.
fn parse_inline(run: &[String]) -> Result<Vec<ProcessSpec>, String> {
  let mut result = Vec::new();
  for entry in run {
    match core::procfile::parse_line(entry) {
      Ok(def) => result.push(def.to_spec()),
      Err(reason) => {
        return Err(format!(
          "Invalid --run entry {:?}: it {}\nExpected: \"name: command\" or \"name(flags): command\"",
          entry, reason
        ));
      }
    }
  }
  Ok(result)
}

/// Optional processes the command line opted into: anything named positionally
/// (which already narrowed the Procfile) plus every -e/--enable argument.
fn opted_in_names(start: &RunOptions) -> Vec<String> {
  let mut names = start.names.clone();
  for name in &start.enable {
    if !names.contains(name) {
      names.push(name.clone());
    }
  }
  names
}

/// Check whether -c/--config was explicitly passed on the CLI.
fn config_explicitly_set() -> bool {
  std::env::args().any(|a| a == "-c" || a == "--config")
}

/// Read a Procfile and parse it into owned process definitions.
fn read_and_parse(config_path: &str) -> Result<Vec<ProcessSpec>, ExitCode> {
  let config = fs::read_to_string(config_path).map_err(|err| {
    eprintln!("{}: {}", config_path, format_io_error(&err));
    ExitCode::from(2)
  })?;

  core::procfile::parse(&config, &[])
    .map(|procs| procs.iter().map(|def| def.to_spec()).collect())
    .map_err(|err| {
      eprintln!("{}\n{}", config_path, err);
      ExitCode::from(2)
    })
}

/// Spawn the web UI server if the feature is enabled and a port is requested.
#[cfg(feature = "web")]
fn maybe_spawn_web(opts: &RunOptions, log_tx: &broadcast::Sender<crate::core::LogEvent>) {
  if let Some(port) = opts.web {
    let rx = log_tx.subscribe();
    let resolved_port = if port == 0 { web::port_for_cwd() } else { port };
    let no_system = opts.no_system;
    tokio::spawn(async move {
      web::start(rx, resolved_port, no_system).await;
    });
  }
}

/// Spawn the stdout consumer task if not silent.
fn spawn_stdout(
  opts: &RunOptions,
  name_width: usize,
  log_tx: &broadcast::Sender<crate::core::LogEvent>,
) -> Option<tokio::task::JoinHandle<()>> {
  spawn_stdout_interactive(opts, name_width, &[], log_tx, None, None, None)
}

fn spawn_stdout_interactive(
  opts: &RunOptions,
  name_width: usize,
  muted: &[ProcessSpec],
  log_tx: &broadcast::Sender<crate::core::LogEvent>,
  name_width_rx: Option<watch::Receiver<usize>>,
  display_rx: Option<mpsc::UnboundedReceiver<input::DisplayCommand>>,
  modal_active: Option<watch::Receiver<bool>>,
) -> Option<tokio::task::JoinHandle<()>> {
  if opts.silent {
    return None;
  }
  let rx = log_tx.subscribe();
  Some(tokio::spawn(stdout::run(
    rx,
    stdout::StdoutConfig {
      timestamps: opts.timestamps,
      compact: opts.compact,
      no_system: opts.no_system,
      name_width,
      muted: muted
        .iter()
        .filter(|spec| spec.flags.muted)
        .map(|spec| spec.name.clone())
        .collect(),
      name_width_rx,
      display_rx,
      modal_active,
    },
  )))
}

#[tokio::main(flavor = "current_thread")]
pub async fn main() -> ExitCode {
  let args = Args::parse();

  match args.command {
    Some(Command::Check { config }) => cmd_check(&config),
    Some(Command::List { config }) => cmd_list(&config),
    Some(Command::Start(opts)) => cmd_start(opts).await,
    None => cmd_start(args.run).await,
  }
}

fn cmd_check(config_path: &str) -> ExitCode {
  match read_and_parse(config_path) {
    Ok(processes) => {
      eprintln!("{}: {} processes OK", config_path, processes.len());
      for spec in &processes {
        eprintln!("  {}{}: {}", spec.name, spec.flags.suffix(), spec.cmd);
      }
      ExitCode::SUCCESS
    }
    Err(code) => code,
  }
}

fn cmd_list(config_path: &str) -> ExitCode {
  match read_and_parse(config_path) {
    Ok(processes) => {
      // Flags make the first column much wider than a name alone, so size it
      // to the entries rather than to a fixed guess.
      let labels: Vec<String> = processes
        .iter()
        .map(|spec| format!("{}{}", spec.name, spec.flags.suffix()))
        .collect();
      let width = labels.iter().map(String::len).max().unwrap_or(0).max(20);
      for (label, spec) in labels.iter().zip(&processes) {
        println!("{:<width$} {}", label, spec.cmd, width = width);
      }
      ExitCode::SUCCESS
    }
    Err(code) => code,
  }
}

async fn cmd_start(start: RunOptions) -> ExitCode {
  let piped = !std::io::stdin().is_terminal();
  if piped {
    return cmd_start_pipe(start).await;
  }

  let has_inline = !start.run.is_empty();
  let has_explicit_config = config_explicitly_set();

  let inline = match parse_inline(&start.run) {
    Ok(v) => v,
    Err(err) => {
      eprintln!("{}", err);
      return 2.into();
    }
  };

  let use_file = has_explicit_config || !has_inline;

  let mut all_processes: Vec<ProcessSpec> = Vec::new();
  if use_file {
    let config = match fs::read_to_string(&start.config) {
      Ok(c) => c,
      Err(err) => {
        eprintln!("{}: {}", start.config, format_io_error(&err));
        return 2.into();
      }
    };

    match core::procfile::parse(&config, &start.names) {
      Ok(p) => all_processes = p.iter().map(|def| def.to_spec()).collect(),
      Err(err) => {
        eprintln!("Error parsing Procfile\n{}", err);
        return 2.into();
      }
    }
  }

  all_processes.extend(inline);

  if all_processes.is_empty() {
    eprintln!("No processes to run");
    return 2.into();
  }

  let opted_in = opted_in_names(&start);
  let unknown: Vec<&str> = start
    .enable
    .iter()
    .filter(|name| !all_processes.iter().any(|spec| &&spec.name == name))
    .map(String::as_str)
    .collect();
  if !unknown.is_empty() {
    eprintln!("Unknown process: {}", unknown.join(", "));
    return 2.into();
  }

  // Interactive mode needs a real terminal on both ends to draw the modal.
  let interactive = std::io::stdout().is_terminal() && !start.no_interactive;

  // Headless there is no menu to bring an optional process up later, so one
  // nobody asked for is simply not part of this run.
  if !interactive {
    all_processes
      .retain(|spec| !spec.flags.optional || start.enable_all || opted_in.contains(&spec.name));

    if all_processes.is_empty() {
      eprintln!(
        "No processes to run — every matching process is optional. \
         Enable one with -e/--enable <name>, or all of them with -E/--enable-all."
      );
      return 2.into();
    }
  }

  let (log_tx, _) = broadcast::channel(16384);

  let mut manager = match ProcessManager::new(
    &all_processes,
    start.effective_on_exit(interactive),
    log_tx.clone(),
  ) {
    Ok(m) => m,
    Err(err) => {
      eprintln!("{}", err);
      return 2.into();
    }
  };

  if start.enable_all {
    manager.enable_all();
  } else {
    // Naming a process on the command line is itself a request to run it,
    // optional or not.
    manager.enable(&opted_in);
  }

  // Set up interactive mode channels
  let (name_width_rx, display_rx, modal_rx, raw_guard) = if interactive {
    let (input_tx, input_rx) = mpsc::unbounded_channel();
    let (display_tx, display_rx) = mpsc::unbounded_channel();
    let (key_tx, key_rx) = mpsc::unbounded_channel();
    let (modal_tx, modal_rx) = watch::channel(false);
    let (resources_tx, resources_rx) = watch::channel(core::resources::ResourceMap::new());
    let name_width_rx = manager.name_width_watch();
    let snapshot_rx = manager.snapshot_watch();
    manager.set_input_rx(input_rx);

    // Install panic hook and enable raw mode
    input::install_panic_hook();
    let guard = input::RawModeGuard::new().expect("Failed to enable raw terminal mode");

    // The resource sampler blocks on syscalls, so it gets its own OS thread
    // rather than a tokio task — this runtime is single-threaded.
    std::thread::spawn({
      let snapshot_rx = snapshot_rx.clone();
      move || core::resources::run_sampler(snapshot_rx, resources_tx)
    });

    // Spawn the terminal event reader and the key-handling loop
    tokio::spawn(input::read_events(key_tx));
    tokio::spawn(input::run(
      input_tx,
      display_tx,
      modal_tx,
      snapshot_rx,
      resources_rx,
      key_rx,
    ));

    (
      Some(name_width_rx),
      Some(display_rx),
      Some(modal_rx),
      Some(guard),
    )
  } else {
    (None, None, None, None)
  };

  let stdout_handle = spawn_stdout_interactive(
    &start,
    manager.name_width(),
    &all_processes,
    &log_tx,
    name_width_rx,
    display_rx,
    modal_rx,
  );

  if interactive {
    let _ = log_tx.send(crate::core::LogEvent {
      process: "system".to_string(),
      color: colored::Color::White,
      line: "Interactive mode — press G to open the menu".to_string(),
      system: true,
      reply: false,
    });
  }

  #[cfg(feature = "web")]
  maybe_spawn_web(&start, &log_tx);

  let code = manager.start(interactive).await;
  drop(manager);

  // Drop raw mode guard explicitly (restores terminal)
  drop(raw_guard);

  #[cfg(feature = "web")]
  if start.web.is_some() {
    drop(log_tx);
    if let Some(handle) = stdout_handle {
      let _ = handle.await;
    }
    eprintln!("SSE server still running. Press Ctrl+C to quit.");
    let _ = tokio::signal::ctrl_c().await;
    return code.into();
  }

  if interactive {
    // Close broadcast so stdout consumer drains remaining messages then exits.
    drop(log_tx);
    if let Some(handle) = stdout_handle {
      // Give stdout a moment to flush final messages (Stopped, etc.),
      // but don't wait forever — the input task's spawn_blocking holds a thread.
      let _ = tokio::time::timeout(std::time::Duration::from_millis(100), handle).await;
    }
    const FAREWELLS: &[&str] = &[
      "All processes down. Until next time!",
      "Clean shutdown. Go grab a coffee.",
      "Nothing left running. See you around!",
      "Processes stopped. Have a great one!",
      "That's a wrap. Happy coding!",
      "All quiet on the process front.",
      "Signed off. Catch you later!",
      "Everything's tucked in. Goodnight!",
      "Done and dusted. Take it easy!",
      "Fin. May your deploys be boring.",
    ];
    let idx = (std::process::id() as usize) % FAREWELLS.len();
    println!("{}", FAREWELLS[idx]);
    std::process::exit(code as i32);
  }

  drop(log_tx);

  if let Some(handle) = stdout_handle {
    let _ = handle.await;
  }

  code.into()
}

async fn cmd_start_pipe(start: RunOptions) -> ExitCode {
  let (log_tx, _) = broadcast::channel(16384);

  let stdout_handle = spawn_stdout(&start, "stdin".len(), &log_tx);

  #[cfg(feature = "web")]
  maybe_spawn_web(&start, &log_tx);

  let exit_code = pipe::run(log_tx).await;

  if let Some(handle) = stdout_handle {
    let _ = handle.await;
  }

  exit_code.into()
}

fn format_io_error(err: &std::io::Error) -> String {
  match err.kind() {
    std::io::ErrorKind::NotFound => "file not found".to_string(),
    std::io::ErrorKind::InvalidData => "not a valid text file".to_string(),
    _ => err.to_string(),
  }
}
