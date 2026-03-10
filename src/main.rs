//! A process manager that runs commands defined in a Procfile.
//!
//! Each process is spawned inside its own PTY, and output lines are
//! multiplexed to stdout with a colored name prefix.
//!
//! Also supports pipe mode: `my_command | procfile` reads stdin and
//! displays it through the same stdout/web consumers.

use crate::core::manager::{ProcessManager, RunningMode};
use clap::Parser;
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
  Names starting with '_' are disabled by default.
  Use --include to enable them.

Examples:
  procfile
  procfile -c Procfile.dev -m restart
  procfile -x web -x worker
  procfile -f Procfile.dev -T")]
struct Args {
  /// Path to the Procfile
  #[arg(
    short,
    long,
    alias = "file",
    short_alias = 'f',
    default_value = DEFAULT_CONFIG
  )]
  config: String,

  /// Running mode
  #[arg(value_enum, short, long, default_value_t = RunningMode::Exit)]
  mode: RunningMode,

  /// Exclude processes by name (can be repeated)
  #[arg(short = 'x', long)]
  exclude: Vec<String>,

  /// Include a _-prefixed (disabled) process by name (can be repeated)
  #[arg(short, long, alias = "enable")]
  include: Vec<String>,

  /// Prefix each line with a timestamp
  #[arg(short = 'T', long)]
  timestamps: bool,

  /// Hide process names, show only colored |
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

#[tokio::main(flavor = "current_thread")]
pub async fn main() -> ExitCode {
  let args = Args::parse();
  let piped = !std::io::stdin().is_terminal();

  // Create the broadcast channel for log events.
  // Core sends via tx; stdout and web receive via rx.
  let (log_tx, _) = broadcast::channel(16384);

  // Subscribe consumers before passing tx to the manager.
  let stdout_rx = if !args.silent {
    Some(log_tx.subscribe())
  } else {
    None
  };

  #[cfg(feature = "web")]
  let web_rx = if args.web.is_some() {
    Some(log_tx.subscribe())
  } else {
    None
  };

  let name_width = if piped { "stdin".len() } else { 0 };

  // In Procfile mode, parse the config and build the manager.
  let mut manager = if !piped {
    let config = match fs::read_to_string(&args.config) {
      Ok(config) => config,
      Err(err) => {
        match err.kind() {
          std::io::ErrorKind::NotFound => eprintln!("{}: file not found", args.config),
          std::io::ErrorKind::InvalidData => eprintln!("{}: not a valid text file", args.config),
          _ => eprintln!("Error reading {}: {}", args.config, err),
        }

        return 2.into();
      }
    };

    match ProcessManager::from_string(
      &config,
      args.mode,
      &args.exclude,
      &args.include,
      log_tx.clone(),
    ) {
      Ok(manager) => Some(manager),
      Err(err) => {
        eprintln!("Error parsing Procfile\n{}", err);
        return 2.into();
      }
    }
  } else {
    None
  };

  let actual_name_width = manager
    .as_ref()
    .map(|m| m.name_width())
    .unwrap_or(name_width);

  // Start stdout consumer (unless silent)
  let stdout_handle = stdout_rx.map(|rx| {
    tokio::spawn(stdout::run(
      rx,
      stdout::StdoutConfig {
        timestamps: args.timestamps,
        compact: args.compact,
        name_width: actual_name_width,
      },
    ))
  });

  // Start web consumer (if enabled)
  #[cfg(feature = "web")]
  if let Some(port) = args.web {
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

  // Run: pipe mode reads stdin, Procfile mode runs the process manager.
  let exit_code = if piped {
    pipe::run(log_tx).await
  } else {
    let code = manager.as_mut().unwrap().start().await;
    drop(manager);
    code
  };

  if let Some(handle) = stdout_handle {
    let _ = handle.await;
  }

  exit_code.into()
}
