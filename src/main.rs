use crate::process_manager::{ProcessManager, RunningMode};
use clap::Parser;
use std::fs;

mod ansi;
mod process;
mod process_manager;
mod signal;

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
  #[arg(short, long, alias = "file", short_alias = 'f', default_value = "Procfile")]
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
}

#[tokio::main(flavor = "current_thread")]
pub async fn main() {
  let args = Args::parse();

  let config = match fs::read_to_string(&args.config) {
    Ok(config) => config,
    Err(err) => {
      match err.kind() {
        std::io::ErrorKind::NotFound => eprintln!("{}: file not found", args.config),
        std::io::ErrorKind::InvalidData => eprintln!("{}: not a valid text file", args.config),
        _ => eprintln!("Error reading {}: {}", args.config, err),
      }
      return;
    }
  };

  let mut manager = match ProcessManager::from_string(&config, args.mode, &args.exclude, &args.include, args.timestamps) {
    Ok(manager) => manager,
    Err(err) => {
      eprintln!("Error parsing Procfile\n{}", err);
      return;
    }
  };

  manager.start().await;
}
