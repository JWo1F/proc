use crate::process_manager::ProcessManager;
use std::fs;
use tokio::select;

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
  
  let mut manager = {
    let manager = ProcessManager::from_string(&config);

    match manager {
      Err(err) => {
        eprintln!("Error parsing Procfile: {}", err);
        return;
      }

      Ok(manager) => {
        manager
      }
    }
  };

  loop {
    select! {
      line = manager.read_line() => {
        match line {
          Some(line) => {
            println!("{}", line);
          }
          
          None => {
            break;
          }
        }
      }
      
      _ = tokio::signal::ctrl_c() => {
        println!("Got Ctrl-C!");
        manager.stop().await;
      }
    }
  }
}
