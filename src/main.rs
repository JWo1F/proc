use crate::process_manager::{ProcessManager, RunningMode};
use std::fs;
use std::process::exit;
use clap::Parser;

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
}

#[tokio::main(flavor = "current_thread")]
pub async fn main() {
  let args = Args::parse();
  
  let config = {
    let conf = fs::read(args.config);

    if conf.is_err() {
      eprintln!("Error reading Procfile: {}", conf.unwrap_err());
      return;
    }
    
    conf.unwrap()
  };
  
  let config = String::from_utf8_lossy(&config);
  let manager = ProcessManager::from_string(&config, args.mode, args.exclude);

  match manager {
    Err(err) => {
      eprintln!("Error parsing Procfile\n{}", err);
      return;
    }

    Ok(mut manager) => {
      manager.start().await;
    }
  }
}
