use colored::Color;
use pty_process::{Command, Pty};
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::Child;
use crate::process_manager::ProcessManager;

pub struct Process {
  pub name: String,
  pub cmd: String,
  pub reader: Option<Lines<BufReader<Pty>>>,
  pub child: Option<Child>,
  pub color: Color,
}

pub enum ReadResult {
  Some(String),
  EOF,
  Err(String),
}

impl Process {
  pub fn new(name: &str, cmd: &str, color: Color) -> Self {
    let mut proc = Self {
      name: name.to_string(),
      cmd: cmd.to_string(),
      reader: None,
      child: None,
      color,
    };

    proc.start();

    proc
  }

  pub fn start(&mut self) {
    let (pty, pts) = pty_process::open().unwrap();
    let cmd = Command::new("/bin/bash").arg("-c").arg(&self.cmd);
    let child = cmd.spawn(pts).unwrap();
    
    self.child = Some(child);
    self.reader = Some(BufReader::new(pty).lines());
  }

  pub async fn read_line(&mut self) -> ReadResult {
    if let Some(ref mut reader) = self.reader {
      let res = reader.next_line().await;

      match res {
        Ok(None) => ReadResult::EOF,
        Ok(Some(buf)) => ReadResult::Some(buf),
        Err(err) => ReadResult::Err(format!("{}", err)),
      }
    } else {
      ReadResult::Err("No reader available".to_string())
    }
  }
  
  pub fn msg_spawn(&self, manager: &ProcessManager) {
    let pid =  self.child.as_ref().unwrap().id().unwrap();
    let msg = format!("Spawned, pid: {}", pid);
    println!("{}", manager.compose_line(self, &msg));
  }
}
