use colored::Color;
use pty_process::{Command, Pty};
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::Child;

const SHELL_ENV: &str = "SHELL";
const DEFAULT_SHELL: &str = "/bin/sh";

pub struct Process {
  pub(crate) name: String,
  cmd: String,
  pub(crate) child: Option<Child>,
  pub(crate) color: Color,
}

impl Process {
  pub fn new(name: &str, cmd: &str, color: Color) -> Self {
    Self {
      name: name.to_string(),
      cmd: cmd.to_string(),
      child: None,
      color,
    }
  }

  pub fn start(&mut self) -> Result<Lines<BufReader<Pty>>, String> {
    let (pty, pts) = pty_process::open().map_err(|e| format!("Failed to open PTY: {}", e))?;
    let shell = std::env::var(SHELL_ENV).unwrap_or_else(|_| DEFAULT_SHELL.to_string());
    let cmd = Command::new(shell).arg("-c").arg(&self.cmd);
    let child = cmd.spawn(pts).map_err(|e| format!("Failed to spawn process: {}", e))?;

    self.child = Some(child);

    Ok(BufReader::new(pty).lines())
  }

  pub fn pid(&self) -> Option<u32> {
    self.child.as_ref().and_then(|c| c.id())
  }
}
