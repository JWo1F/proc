//! A process manager that runs commands defined in a Procfile.
//!
//! Each process is spawned inside its own PTY, and output lines are
//! multiplexed to stdout with a colored name prefix.
//!
//! Also supports pipe mode: `my_command | procfile` reads stdin and
//! displays it through the same stdout/web consumers.

use crate::core::manager::{OnExit, ProcessManager};
use clap::{Parser, Subcommand};
use std::fs;
use std::io::IsTerminal;
use std::process::ExitCode;
use tokio::sync::{broadcast, mpsc, watch};

mod core;
mod pipe;
mod input;
mod stdout;
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

  /// Behavior when a process exits
  #[arg(value_enum, long, default_value_t = OnExit::Stop)]
  on_exit: OnExit,

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

  /// Enable interactive command prompt
  #[arg(short = 'i', long)]
  interactive: bool,

  /// Start SSE server (optional port, default: derived from folder name)
  #[cfg(feature = "web")]
  #[arg(short = 'w', long, num_args = 0..=1, default_missing_value = "0")]
  web: Option<u16>,
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
#[command(after_help = "\
Procfile format:
  <name>: <command>

  Lines starting with '#' are comments.

Examples:
  procfile
  procfile web worker
  procfile -r 'extra: sidekiq'
  procfile check
  procfile list")]
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

/// Parse --run "name: command" into (name, command), reusing Procfile syntax.
fn parse_inline(run: &[String]) -> Result<Vec<(&str, &str)>, String> {
  let mut result = Vec::new();
  for entry in run {
    let Some((name, cmd)) = entry.split_once(':') else {
      return Err(format!(
        "Invalid --run format: {:?}\nExpected: \"name: command\"",
        entry
      ));
    };
    let name = name.trim();
    let cmd = cmd.trim();
    if name.is_empty() || cmd.is_empty() {
      return Err(format!(
        "Invalid --run format: {:?}\nExpected: \"name: command\"",
        entry
      ));
    }
    result.push((name, cmd));
  }
  Ok(result)
}

/// Check whether -c/--config was explicitly passed on the CLI.
fn config_explicitly_set() -> bool {
  std::env::args().any(|a| a == "-c" || a == "--config")
}

/// Read a Procfile and parse it, returning owned (name, command) pairs.
fn read_and_parse(config_path: &str) -> Result<Vec<(String, String)>, ExitCode> {
  let config = fs::read_to_string(config_path).map_err(|err| {
    eprintln!("{}: {}", config_path, format_io_error(&err));
    ExitCode::from(2)
  })?;

  core::procfile::parse(&config, &[])
    .map(|procs| {
      procs
        .into_iter()
        .map(|(n, c)| (n.to_string(), c.to_string()))
        .collect()
    })
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
    let resolved_port = if port == 0 {
      web::port_for_cwd()
    } else {
      port
    };
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
      interactive: None,
      name_width_rx: None,
    },
  )))
}

fn spawn_stdout_interactive(
  opts: &RunOptions,
  name_width: usize,
  log_tx: &broadcast::Sender<crate::core::LogEvent>,
  buffer_rx: Option<watch::Receiver<String>>,
  name_width_rx: Option<watch::Receiver<usize>>,
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
      interactive: buffer_rx,
      name_width_rx,
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
      for (name, cmd) in &processes {
        eprintln!("  {}: {}", name, cmd);
      }
      ExitCode::SUCCESS
    }
    Err(code) => code,
  }
}

fn cmd_list(config_path: &str) -> ExitCode {
  match read_and_parse(config_path) {
    Ok(processes) => {
      for (name, cmd) in &processes {
        println!("{:<20} {}", name, cmd);
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

  let mut all_processes: Vec<(String, String)> = Vec::new();
  if use_file {
    let config = match fs::read_to_string(&start.config) {
      Ok(c) => c,
      Err(err) => {
        eprintln!("{}: {}", start.config, format_io_error(&err));
        return 2.into();
      }
    };

    match core::procfile::parse(&config, &start.names) {
      Ok(p) => {
        all_processes = p
          .into_iter()
          .map(|(n, c)| (n.to_string(), c.to_string()))
          .collect()
      }
      Err(err) => {
        eprintln!("Error parsing Procfile\n{}", err);
        return 2.into();
      }
    }
  }

  for (name, cmd) in inline {
    all_processes.push((name.to_string(), cmd.to_string()));
  }

  if all_processes.is_empty() {
    eprintln!("No processes to run");
    return 2.into();
  }

  let process_refs: Vec<(&str, &str)> = all_processes
    .iter()
    .map(|(n, c)| (n.as_str(), c.as_str()))
    .collect();

  let (log_tx, _) = broadcast::channel(16384);

  let mut manager = match ProcessManager::new(&process_refs, start.on_exit, log_tx.clone()) {
    Ok(m) => m,
    Err(err) => {
      eprintln!("{}", err);
      return 2.into();
    }
  };

  let interactive = start.interactive;

  // Set up interactive mode channels
  let (buffer_rx, name_width_rx, raw_guard) = if interactive {
    let (itx, irx) = mpsc::unbounded_channel();
    let (btx, brx) = watch::channel(String::new());
    let nw_rx = manager.name_width_watch();
    manager.set_input_rx(irx);

    // Install panic hook and enable raw mode
    input::install_panic_hook();
    let guard = input::RawModeGuard::new().expect("Failed to enable raw terminal mode");

    // Spawn the input reading task
    let log_tx_clone = log_tx.clone();
    tokio::spawn(input::run(itx, btx, log_tx_clone));

    (Some(brx), Some(nw_rx), Some(guard))
  } else {
    (None, None, None)
  };

  let stdout_handle = spawn_stdout_interactive(&start, manager.name_width(), &log_tx, buffer_rx, name_width_rx);

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
