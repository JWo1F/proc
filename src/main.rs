//! A process manager that runs commands defined in a Procfile.
//!
//! Each process is spawned inside its own PTY, and output lines are
//! multiplexed to stdout with a colored name prefix.

use crate::core::manager::{ProcessManager, RunningMode};
use clap::Parser;
use std::fs;
use std::process::ExitCode;
use tokio::sync::broadcast;

mod core;
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

  /// Start web UI (optional port, default: derived from folder name)
  #[cfg(feature = "web")]
  #[arg(short = 'w', long, num_args = 0..=1, default_missing_value = "0")]
  web: Option<u16>,
}

#[tokio::main(flavor = "current_thread")]
pub async fn main() -> ExitCode {
  let args = Args::parse();

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

  // Create the broadcast channel for log events.
  // Core sends via tx; stdout and web receive via rx.
  let (log_tx, _) = broadcast::channel(16384);

  // Subscribe consumers before passing tx to the manager.
  let stdout_rx = log_tx.subscribe();

  #[cfg(feature = "web")]
  let web_rx = if args.web.is_some() {
    Some(log_tx.subscribe())
  } else {
    None
  };

  let mut manager = match ProcessManager::from_string(
    &config,
    args.mode,
    &args.exclude,
    &args.include,
    log_tx,
  ) {
    Ok(manager) => manager,
    Err(err) => {
      eprintln!("Error parsing Procfile\n{}", err);
      return 2.into();
    }
  };

  // Start stdout consumer
  let stdout_handle = tokio::spawn(stdout::run(
    stdout_rx,
    stdout::StdoutConfig {
      timestamps: args.timestamps,
      compact: args.compact,
      name_width: manager.name_width(),
    },
  ));

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

  // Run the core event loop (blocks until all processes exit)
  let exit_code = manager.start().await;

  // Drop the manager to close the broadcast channel,
  // allowing consumers to drain and finish.
  drop(manager);
  let _ = stdout_handle.await;

  exit_code.into()
}
