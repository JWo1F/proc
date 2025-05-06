use colored::Color;
use pty_process::{Command, Pty};
use terminal_size::{terminal_size, Height, Width};
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::Child;

pub struct Process {
  pub name: String,
  pub reader: Lines<BufReader<Pty>>,
  pub child: Child,
  pub color: Color,
}

pub enum ReadResult {
  Some(String),
  EOF,
  Err(String),
}

impl Process {
  pub fn new(name: &str, cmd: &str, color: Color) -> Self {
    let (pty, pts) = pty_process::open().unwrap();
    let (Width(w), Height(h)) = terminal_size().unwrap();

    pty.resize(pty_process::Size::new(w, h)).unwrap();

    let cmd = Command::new("/bin/bash")
      .arg("-c")
      .arg(cmd)
      .env("TERM", "xterm-256color");

    let child = cmd.spawn(pts).unwrap();
    
    println!("{} spawned ({:?})", name, child.id());
    
    Self {
      name: name.to_string(),
      reader: BufReader::new(pty).lines(),
      child,
      color,
    }
  }

  pub async fn read_line(&mut self) -> ReadResult {
    let res = self.reader.next_line().await;

    match res {
      Ok(None) => ReadResult::EOF,
      Ok(Some(buf)) => ReadResult::Some(buf),
      Err(err) => ReadResult::Err(format!("{}", err)),
    }
  }
}
