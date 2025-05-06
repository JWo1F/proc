use crate::process_manager::ProcessManager;
use std::fs;

mod process;
mod process_manager;
mod ansi;
mod signal;

#[tokio::main(flavor = "current_thread")]
pub async fn main() {
  let config = {
    let conf = fs::read("./Procfile");

    if conf.is_err() {
      eprintln!("Error reading Procfile: {}", conf.unwrap_err());
      return;
    }
    
    conf.unwrap()
  };
  
  let config = String::from_utf8_lossy(&config);
  let manager = ProcessManager::from_string(&config);

  match manager {
    Err(err) => {
      eprintln!("Error parsing Procfile: {}", err);
      return;
    }

    Ok(mut manager) => {
      manager.start().await;
    }
  }
}
