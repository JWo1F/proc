use colored::Color;
use pty_process::{Command, Pty};
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::Child;

pub struct Process {
  pub(crate) name: String,
  cmd: String,
  reader: Option<Lines<BufReader<Pty>>>,
  pub(crate) child: Option<Child>,
  pub(crate) color: Color,
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
        Ok(Some(buf)) => {
          let stripped = crate::ansi::strip_ansi_except_colors(buf.as_bytes());
          ReadResult::Some(String::from_utf8_lossy(&stripped).into_owned())
        }
        Err(err) => ReadResult::Err(format!("{}", err)),
      }
    } else {
      ReadResult::Err("No reader available".to_string())
    }
  }

  pub fn pid(&self) -> Option<u32> {
    self.child.as_ref().and_then(|c| c.id())
  }
}
