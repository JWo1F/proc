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
use tokio::sync::broadcast;

mod core;
mod pipe;
mod stdout;
#[cfg(feature = "web")]
mod web;

const DEFAULT_CONFIG: &str = "Procfile";

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

  /// Start web UI (optional port, default: derived from folder name)
  #[cfg(feature = "web")]
  #[arg(short = 'w', long, num_args = 0..=1, default_missing_value = "0")]
  web: Option<u16>,
}

#[derive(Subcommand, Debug)]
enum Command {
  /// Start processes (default — same as running without a subcommand)
  Start(StartArgs),

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

#[derive(Parser, Debug)]
struct StartArgs {
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

  /// Start web UI (optional port, default: derived from folder name)
  #[cfg(feature = "web")]
  #[arg(short = 'w', long, num_args = 0..=1, default_missing_value = "0")]
  web: Option<u16>,
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

#[tokio::main(flavor = "current_thread")]
pub async fn main() -> ExitCode {
  let args = Args::parse();

  match args.command {
    Some(Command::Check { config }) => cmd_check(&config),
    Some(Command::List { config }) => cmd_list(&config),
    Some(Command::Start(start)) => cmd_start(start).await,
    None => {
      // Default: treat top-level args as "start".
      let start = StartArgs {
        names: args.names,
        config: args.config,
        run: args.run,
        on_exit: args.on_exit,
        timestamps: args.timestamps,
        compact: args.compact,
        silent: args.silent,
        #[cfg(feature = "web")]
        web: args.web,
      };
      cmd_start(start).await
    }
  }
}

fn cmd_check(config_path: &str) -> ExitCode {
  let config = match fs::read_to_string(config_path) {
    Ok(c) => c,
    Err(err) => {
      eprintln!("{}: {}", config_path, format_io_error(&err));
      return 2.into();
    }
  };

  match core::procfile::parse(&config, &[]) {
    Ok(processes) => {
      eprintln!("{}: {} processes OK", config_path, processes.len());
      for (name, cmd) in &processes {
        eprintln!("  {}: {}", name, cmd);
      }
      ExitCode::SUCCESS
    }
    Err(err) => {
      eprintln!("{}\n{}", config_path, err);
      2.into()
    }
  }
}

fn cmd_list(config_path: &str) -> ExitCode {
  let config = match fs::read_to_string(config_path) {
    Ok(c) => c,
    Err(err) => {
      eprintln!("{}: {}", config_path, format_io_error(&err));
      return 2.into();
    }
  };

  match core::procfile::parse(&config, &[]) {
    Ok(processes) => {
      for (name, cmd) in &processes {
        println!("{:<20} {}", name, cmd);
      }
      ExitCode::SUCCESS
    }
    Err(err) => {
      eprintln!("{}\n{}", config_path, err);
      2.into()
    }
  }
}

async fn cmd_start(start: StartArgs) -> ExitCode {
  let piped = !std::io::stdin().is_terminal();
  if piped {
    return cmd_start_pipe(start).await;
  }

  let has_inline = !start.run.is_empty();
  let has_explicit_config = config_explicitly_set();

  // Parse inline --run processes.
  let inline = match parse_inline(&start.run) {
    Ok(v) => v,
    Err(err) => {
      eprintln!("{}", err);
      return 2.into();
    }
  };

  // Determine whether to read the Procfile.
  // Skip file if only --run is used without explicit -c.
  let use_file = has_explicit_config || !has_inline;

  let mut file_processes = Vec::new();
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
        file_processes = p
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

  // Merge file + inline processes.
  let mut all_processes: Vec<(String, String)> = file_processes;
  for (name, cmd) in inline {
    all_processes.push((name.to_string(), cmd.to_string()));
  }

  if all_processes.is_empty() {
    eprintln!("No processes to run");
    return 2.into();
  }

  // Build refs for ProcessManager.
  let process_refs: Vec<(&str, &str)> = all_processes
    .iter()
    .map(|(n, c)| (n.as_str(), c.as_str()))
    .collect();

  let (log_tx, _) = broadcast::channel(16384);

  let stdout_rx = if !start.silent {
    Some(log_tx.subscribe())
  } else {
    None
  };

  #[cfg(feature = "web")]
  let web_rx = if start.web.is_some() {
    Some(log_tx.subscribe())
  } else {
    None
  };

  let mut manager = match ProcessManager::new(&process_refs, start.on_exit, log_tx.clone()) {
    Ok(m) => m,
    Err(err) => {
      eprintln!("{}", err);
      return 2.into();
    }
  };

  let stdout_handle = stdout_rx.map(|rx| {
    tokio::spawn(stdout::run(
      rx,
      stdout::StdoutConfig {
        timestamps: start.timestamps,
        compact: start.compact,
        name_width: manager.name_width(),
      },
    ))
  });

  #[cfg(feature = "web")]
  if let Some(port) = start.web {
    let rx = web_rx.unwrap();
    let resolved_port = if port == 0 {
      web::port_for_cwd()
    } else {
      port
    };
    tokio::spawn(async move {
      web::start(rx, resolved_port).await;
    });
  }

  let code = manager.start().await;
  drop(manager);

  if let Some(handle) = stdout_handle {
    let _ = handle.await;
  }

  code.into()
}

async fn cmd_start_pipe(start: StartArgs) -> ExitCode {
  let (log_tx, _) = broadcast::channel(16384);

  let stdout_rx = if !start.silent {
    Some(log_tx.subscribe())
  } else {
    None
  };

  #[cfg(feature = "web")]
  let web_rx = if start.web.is_some() {
    Some(log_tx.subscribe())
  } else {
    None
  };

  let name_width = "stdin".len();

  let stdout_handle = stdout_rx.map(|rx| {
    tokio::spawn(stdout::run(
      rx,
      stdout::StdoutConfig {
        timestamps: start.timestamps,
        compact: start.compact,
        name_width,
      },
    ))
  });

  #[cfg(feature = "web")]
  if let Some(port) = start.web {
    let rx = web_rx.unwrap();
    let resolved_port = if port == 0 {
      web::port_for_cwd()
    } else {
      port
    };
    tokio::spawn(async move {
      web::start(rx, resolved_port).await;
    });
  }

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
