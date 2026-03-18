use colored::Color;
use nix::sys::signal::Signal;
use pty_process::{Command, Pty};
use std::process::ExitStatus;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::Child;

const SHELL_ENV: &str = "SHELL";
const DEFAULT_SHELL: &str = "/bin/sh";
const RESTART_DELAY_MS: u64 = 250;
const RESTART_MAX_DELAY_MS: u64 = 5_000;

/// A single managed process spawned inside a PTY.
pub struct Process {
  /// Display name from the Procfile (used in log prefixes).
  pub(crate) name: String,
  /// Shell command to execute.
  cmd: String,
  /// Handle to the running child, `None` before start or between restarts.
  child: Option<Child>,
  /// Color assigned for this process's log output.
  pub(crate) color: Color,
  /// Number of consecutive restarts (used for linear backoff).
  restart_attempts: u32,
  /// Set by the `restart` command — bypasses backoff on next exit.
  pub(crate) pending_restart: bool,
  /// Set by the `remove` command — skips all mode logic on exit.
  pub(crate) removed: bool,
}

impl Process {
  pub fn new(name: &str, cmd: &str, color: Color) -> Self {
    Self {
      name: name.to_string(),
      cmd: cmd.to_string(),
      child: None,
      color,
      restart_attempts: 0,
      pending_restart: false,
      removed: false,
    }
  }

  /// Spawn the command inside a new PTY via `$SHELL -c <cmd>`.
  /// Returns a line reader over the PTY output.
  pub fn start(&mut self) -> Result<Lines<BufReader<Pty>>, String> {
    let (pty, pts) = pty_process::open().map_err(|e| format!("Failed to open PTY: {}", e))?;
    let shell = std::env::var(SHELL_ENV).unwrap_or_else(|_| DEFAULT_SHELL.to_string());
    let cmd = Command::new(shell).arg("-c").arg(&self.cmd);
    let child = cmd
      .spawn(pts)
      .map_err(|e| format!("Failed to spawn process: {}", e))?;

    self.child = Some(child);

    Ok(BufReader::new(pty).lines())
  }

  /// PID of the running child, or `None` if not currently running.
  pub fn pid(&self) -> Option<u32> {
    self.child.as_ref().and_then(|c| c.id())
  }

  /// Compute the next restart delay using linear backoff:
  /// 250ms, 500ms, 750ms, ... capped at 5s.
  pub fn next_restart_delay(&mut self) -> Duration {
    let delay_ms = RESTART_DELAY_MS
      .saturating_mul(self.restart_attempts as u64 + 1)
      .min(RESTART_MAX_DELAY_MS);

    self.restart_attempts = self.restart_attempts.saturating_add(1);

    Duration::from_millis(delay_ms)
  }

  /// Reset the restart backoff counter to zero.
  pub fn reset_restart_counter(&mut self) {
    self.restart_attempts = 0;
  }

  /// Whether a child process is currently running.
  pub fn is_running(&self) -> bool {
    self.child.is_some()
  }

  /// Send a Unix signal to the process group (negative PID) so that the child
  /// and all its descendants receive it. No-op if the process is not running.
  pub fn signal(&self, signal: Signal) {
    if let Some(child) = &self.child
      && let Some(pid) = child.id()
    {
      let gid = -(pid as i32);

      if let Err(err) = nix::sys::signal::kill(nix::unistd::Pid::from_raw(gid), signal) {
        eprintln!("Error sending signal to process {}: {}", pid, err);
      }
    }
  }

  /// Take the child handle and collect its exit status.
  pub async fn take_exit_status(&mut self) -> Option<ExitStatus> {
    let mut child = self.child.take()?;

    match child.try_wait() {
      Ok(Some(status)) => Some(status),
      Ok(None) => match child.wait().await {
        Ok(status) => Some(status),
        Err(err) => {
          eprintln!("Error waiting process [{}]: {}", self.name, err);
          None
        }
      },
      Err(err) => {
        eprintln!("Error checking process [{}] status: {}", self.name, err);
        None
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn restart_delay_grows_linearly_and_caps() {
    let mut process = Process::new("web", "echo hi", Color::Blue);

    let first = process.next_restart_delay();
    assert_eq!(first, Duration::from_millis(RESTART_DELAY_MS));

    let second = process.next_restart_delay();
    assert_eq!(second, Duration::from_millis(RESTART_DELAY_MS * 2));

    let third = process.next_restart_delay();
    assert_eq!(third, Duration::from_millis(RESTART_DELAY_MS * 3));

    // Drive past the cap
    for _ in 0..50 {
      process.next_restart_delay();
    }

    let capped = process.next_restart_delay();
    assert_eq!(capped, Duration::from_millis(RESTART_MAX_DELAY_MS));
  }

  #[test]
  fn reset_restart_counter_resets_to_zero() {
    let mut process = Process::new("web", "echo hi", Color::Blue);

    // Build up some restart attempts
    process.next_restart_delay();
    process.next_restart_delay();
    process.next_restart_delay();

    process.reset_restart_counter();

    // After reset, first delay should be base delay again
    let delay = process.next_restart_delay();
    assert_eq!(delay, Duration::from_millis(RESTART_DELAY_MS));
  }
}
