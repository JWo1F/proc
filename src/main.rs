use crate::process::ReadResult;
use crate::process_manager::ProcessManager;
use futures::lock::Mutex;
use std::fs;
use std::process::exit;
use std::sync::Arc;

mod process;
mod process_manager;
mod ansi;

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
    
    if manager.is_err() {
      eprintln!("Error parsing Procfile: {:?}", manager.err().unwrap());
      return;
    }
    
    manager.unwrap()
  };
  
  manager.spawn();
  
  let manager = Arc::new(Mutex::new(manager));
  let disable_manager = manager.clone();
  
  tokio::spawn(async move {
    tokio::signal::ctrl_c().await.unwrap();
    println!("Got Ctrl-C!");
    
    let mut guard = disable_manager.lock().await;
    println!("Lock acquired");
    
    guard.stop().await;
  });

  loop {
    let mut guard = manager.lock().await;
    let (res, proc) = guard.read_line().await;

    match res {
      ReadResult::Some(line) => {
        println!("{}", line);
      }

      ReadResult::EOF => {
        println!("EOF");
      }

      ReadResult::Err(err) => {
        eprintln!("Error: {:?}", err);
        exit(1);
      }
    }
  }
}
