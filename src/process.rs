use colored::Color;
use pty_process::{Command, Pty};
use terminal_size::{terminal_size, Height, Width};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::Child;

pub struct Process {
  pub name: String,
  pub cmd: String,
  pub pair: Option<(Lines<BufReader<Pty>>, Child)>,
  pub color: Color,
}

pub enum ReadResult {
  Some(String),
  EOF,
  Err(String),
}

impl Process {
  pub fn new(name: &str, cmd: &str, color: Color) -> Self {
    Self {
      name: name.to_string(),
      cmd: cmd.to_string(),
      pair: None,
      color,
    }
  }

  pub fn spawn(&mut self) {
    let (pty, pts) = pty_process::open().unwrap();
    let (Width(w), Height(h)) = terminal_size().unwrap();

    pty.resize(pty_process::Size::new(h, w)).unwrap();

    println!("{} spawned", self.name);
    
    let cmd = Command::new("sh")
      .arg("-c")
      .arg(&self.cmd)
      .env("TERM", "xterm-256color");

    let child = cmd.spawn(pts).unwrap();

    self.pair = Some((BufReader::new(pty).lines(), child));
  }

  pub async fn read_line(&mut self) -> ReadResult {
    if let Some((reader, _)) = &mut self.pair {
      let res = reader.next_line().await;
      
      match res {
        Ok(None) => ReadResult::EOF,
        Ok(Some(buf)) => ReadResult::Some(buf),
        Err(_) => ReadResult::Err(format!("{:?}", res)),
      }
    } else {
      ReadResult::Err("Cannot read from closed process".to_string())
    }
  }
}
