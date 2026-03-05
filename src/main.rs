use crate::process_manager::{ProcessManager, RunningMode};
use clap::Parser;
use std::fs;

mod ansi;
mod process;
mod process_manager;
mod signal;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
  /// Path to the Procfile
  #[arg(short, long, default_value = "Procfile")]
  config: String,

  /// Running mode
  #[arg(value_enum, short, long, default_value_t = RunningMode::Exit)]
  mode: RunningMode,

  /// Exclude processes from the Procfile
  #[arg(short = 'x', long)]
  exclude: Vec<String>,

  /// Show timestamps
  #[arg(short = 'T', long)]
  timestamps: bool,
}

#[tokio::main(flavor = "current_thread")]
pub async fn main() {
  let args = Args::parse();

  let config = match fs::read_to_string(&args.config) {
    Ok(config) => config,
    Err(err) => {
      eprintln!("Error reading Procfile: {}", err);
      return;
    }
  };

  let mut manager = match ProcessManager::from_string(&config, args.mode, &args.exclude, args.timestamps) {
    Ok(manager) => manager,
    Err(err) => {
      eprintln!("Error parsing Procfile\n{}", err);
      return;
    }
  };

  manager.start().await;
}
