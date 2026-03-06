use crate::ansi;
use crate::color::color_for_index;
use crate::process::Process;
use crate::procfile;
use clap::ValueEnum;
use colored::Colorize;
use nix::sys::signal::Signal;
use pty_process::Pty;
use std::collections::HashMap;
use std::fmt::Display;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;
use std::time::Duration;
use tokio::io::{BufReader, Lines};
use tokio::sync::mpsc;
use tokio::time::sleep;

const TIMESTAMP_FORMAT: &str = "%H:%M:%S";
const COMPACT_INDICATOR: &str = "▌";
/// How long to wait after SIGINT before sending SIGKILL.
const SIGKILL_GRACE_SECS: u64 = 5;

#[derive(Debug, Clone, Copy, PartialOrd, PartialEq, ValueEnum)]
pub enum RunningMode {
  /// Restart a process when it exits
  Restart,
  /// Stop all processes when any one exits
  Exit,
  /// Ignore process exits, quit when all are done
  Relax,
}

impl Display for RunningMode {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      RunningMode::Restart => write!(f, "Restart"),
      RunningMode::Exit => write!(f, "Exit"),
      RunningMode::Relax => write!(f, "Relax"),
    }
  }
}

/// All events funnelled through a single mpsc channel into the main loop.
enum Event {
  /// A line of output from process `id`.
  Line(usize, String),
  /// Process `id` PTY closed (process exited).
  ProcessEnded(usize),
  /// Delayed restart timer fired for process `id`.
  Restart(usize),
  /// Ctrl+C signal received.
  CtrlC,
  /// Grace period expired — time to SIGKILL remaining processes.
  ForceKill,
}

/// Owns all child processes and drives the main event loop.
///
/// Lifecycle: `from_string()` parses the Procfile and builds the manager,
/// then `start()` spawns everything and blocks until all processes are done.
pub struct ProcessManager {
  /// Current running mode (may switch to Relax during shutdown).
  mode: RunningMode,
  /// Longest process name length, used to align log prefixes.
  name_width: usize,
  /// Active processes keyed by their index from the Procfile.
  processes: HashMap<usize, Process>,
  /// Whether to prefix each output line with a timestamp.
  timestamps: bool,
  /// Compact mode: show a colored block instead of the process name.
  compact: bool,
  /// Sender half of the event channel — cloned into background tasks.
  tx: mpsc::UnboundedSender<Event>,
  /// Receiver half — consumed exclusively by the main event loop.
  rx: mpsc::UnboundedReceiver<Event>,
  /// Set when any process exits with a non-zero status.
  failed: bool,
  /// Set once graceful shutdown has been initiated (SIGINT sent).
  shutting_down: bool,
}

impl ProcessManager {
  /// Parse a Procfile string and build a manager. Does not start processes yet.
  pub fn from_string(
    input: &str,
    mode: RunningMode,
    exclude: &[String],
    include: &[String],
    timestamps: bool,
    compact: bool,
  ) -> Result<Self, String> {
    let parsed = procfile::parse(input, exclude, include)?;

    if parsed.is_empty() {
      return Err("No processes selected after applying include/exclude filters".to_string());
    }

    let name_width = parsed.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
    let (tx, rx) = mpsc::unbounded_channel();

    let processes: HashMap<usize, Process> = parsed
      .iter()
      .enumerate()
      .map(|(i, (name, cmd))| (i, Process::new(name, cmd, color_for_index(i))))
      .collect();

    Ok(Self {
      processes,
      name_width,
      mode,
      timestamps,
      compact,
      tx,
      rx,
      failed: false,
      shutting_down: false,
    })
  }

  /// Read lines from a PTY and forward them as `Event::Line` / `Event::ProcessEnded`.
  fn spawn_reader(id: usize, mut reader: Lines<BufReader<Pty>>, tx: mpsc::UnboundedSender<Event>) {
    tokio::spawn(async move {
      loop {
        match reader.next_line().await {
          Ok(Some(line)) => {
            let stripped = ansi::strip_ansi_except_colors(line.as_bytes());
            let line = String::from_utf8_lossy(&stripped).into_owned();
            if tx.send(Event::Line(id, line)).is_err() {
              break;
            }
          }
          Ok(None) => {
            let _ = tx.send(Event::ProcessEnded(id));
            break;
          }
          Err(err) => {
            eprintln!("Error reading process output: {}", err);
            let _ = tx.send(Event::ProcessEnded(id));
            break;
          }
        }
      }
    });
  }

  /// Schedule a `Event::Restart` after `delay`.
  fn spawn_restart(id: usize, delay: Duration, tx: mpsc::UnboundedSender<Event>) {
    tokio::spawn(async move {
      sleep(delay).await;
      let _ = tx.send(Event::Restart(id));
    });
  }

  /// Listen for Ctrl+C in a loop (each signal sends a new `Event::CtrlC`).
  fn spawn_ctrlc(tx: mpsc::UnboundedSender<Event>) {
    tokio::spawn(async move {
      loop {
        if tokio::signal::ctrl_c().await.is_err() {
          break;
        }

        if tx.send(Event::CtrlC).is_err() {
          break;
        }
      }
    });
  }

  /// One-shot timer: sends `Event::ForceKill` after the SIGKILL grace period.
  fn spawn_force_kill(tx: mpsc::UnboundedSender<Event>) {
    tokio::spawn(async move {
      sleep(Duration::from_secs(SIGKILL_GRACE_SECS)).await;
      let _ = tx.send(Event::ForceKill);
    });
  }

  /// Process IDs in deterministic order (for consistent log output).
  fn sorted_ids(&self) -> Vec<usize> {
    let mut ids: Vec<_> = self.processes.keys().copied().collect();
    ids.sort_unstable();
    ids
  }

  /// Spawn a single process and attach a PTY reader.
  fn start_one(&mut self, id: usize) -> Result<(), String> {
    let reader = {
      let Some(proc) = self.processes.get_mut(&id) else {
        return Ok(());
      };

      proc.start()?
    };

    Self::spawn_reader(id, reader, self.tx.clone());

    if let Some(proc) = self.processes.get(&id) {
      self.log_spawn(proc);
    }

    Ok(())
  }

  /// Start every process. On first failure, stop all already-started ones.
  fn start_all(&mut self) {
    for id in self.sorted_ids() {
      if let Err(err) = self.start_one(id) {
        self.failed = true;

        if let Some(proc) = self.processes.get(&id) {
          eprintln!("{}", self.compose_system_line(proc, &err));
        } else {
          eprintln!("Error starting process {}: {}", id, err);
        }

        self.processes.remove(&id);
        self.processes.retain(|_, process| process.is_running());

        if !self.processes.is_empty() {
          self.stop();
        }
        break;
      }
    }
  }

  /// Build a human-readable exit description: code, signal name, or just "Exited".
  fn format_exit_status(status: Option<&ExitStatus>) -> String {
    let Some(status) = status else {
      return "Exited".to_string();
    };

    if status.success() {
      return "Exited".to_string();
    }

    if let Some(code) = status.code() {
      return format!("Exited with code {}", code);
    }

    if let Some(signal) = status.signal() {
      return format!("Exited from signal {}", signal);
    }

    "Exited".to_string()
  }

  /// React to a process exit according to the current running mode.
  async fn handle_exit(&mut self, id: usize) {
    let (exit_success, exit_message) = {
      let Some(proc) = self.processes.get_mut(&id) else {
        return;
      };

      let status = proc.take_exit_status().await;
      let exit_success = status.as_ref().is_some_and(ExitStatus::success);
      let exit_message = Self::format_exit_status(status.as_ref());

      (exit_success, exit_message)
    };

    match self.mode {
      RunningMode::Restart => {
        if let Some(proc) = self.processes.get(&id) {
          println!("{}", self.compose_system_line(proc, &exit_message));
        }

        // Process stays in the map (child=None) while waiting for the restart timer.
        // If shutdown happens in between, stop() removes childless entries via retain().
        let delay: Option<Duration> = self
          .processes
          .get_mut(&id)
          .map(|proc| proc.next_restart_delay());

        if let Some(delay) = delay {
          if let Some(proc) = self.processes.get(&id) {
            println!(
              "{}",
              self.compose_system_line(proc, &format!("Restarting in {}ms...", delay.as_millis()))
            );
          }

          Self::spawn_restart(id, delay, self.tx.clone());
        }
      }
      RunningMode::Exit => {
        if !exit_success {
          self.failed = true;
        }

        if let Some(proc) = self.processes.get(&id) {
          println!("{}", self.compose_system_line(proc, &exit_message));
        }

        self.processes.remove(&id);
        self.stop();
      }
      RunningMode::Relax => {
        // Don't count failures caused by our own shutdown signals.
        if !exit_success && !self.shutting_down {
          self.failed = true;
        }

        let message = if self.shutting_down {
          "Stopped"
        } else {
          &exit_message
        };

        if let Some(proc) = self.processes.get(&id) {
          println!("{}", self.compose_system_line(proc, message));
        }

        self.processes.remove(&id);
      }
    }
  }

  /// Called when a restart timer fires. Starts the process again (or drops it on failure).
  fn handle_restart(&mut self, id: usize) {
    if self.mode != RunningMode::Restart || self.shutting_down {
      self.processes.remove(&id);
      return;
    }

    if let Err(err) = self.start_one(id) {
      self.failed = true;

      if let Some(proc) = self.processes.get(&id) {
        eprintln!("{}", self.compose_system_line(proc, &err));
      } else {
        eprintln!("Error restarting process {}: {}", id, err);
      }

      self.processes.remove(&id);
    }
  }

  /// Send a signal to every process that still has a running child.
  fn signal_all(&self, signal: Signal, message: &str) {
    for id in self.sorted_ids() {
      let Some(process) = self.processes.get(&id) else {
        continue;
      };

      if process.is_running() {
        process.signal(signal);
        println!("{}", self.compose_system_line(process, message));
      }
    }
  }

  /// Begin graceful shutdown: send SIGINT, schedule a SIGKILL fallback,
  /// and switch mode to Relax so no more restarts happen.
  fn stop(&mut self) {
    if self.shutting_down {
      return;
    }

    self.shutting_down = true;
    self.mode = RunningMode::Relax;
    self.processes.retain(|_, process| process.is_running());
    self.signal_all(Signal::SIGINT, "Stopping...");
    Self::spawn_force_kill(self.tx.clone());
  }

  /// First Ctrl+C starts graceful shutdown; second one sends SIGKILL immediately.
  fn handle_ctrlc(&mut self) {
    if !self.shutting_down {
      println!("\nCtrl+C received, stopping processes...");
      self.stop();
    } else {
      println!("Ctrl+C received again, killing processes...");
      self.signal_all(Signal::SIGKILL, "Killing...");
    }
  }

  /// 0 if all processes exited cleanly, 1 if any failed.
  fn exit_code(&self) -> u8 {
    if self.failed { 1 } else { 0 }
  }

  /// Main event loop. Spawns all processes, then dispatches events
  /// until every process has exited and been removed from the map.
  pub async fn start(&mut self) -> u8 {
    Self::spawn_ctrlc(self.tx.clone());
    self.start_all();

    while !self.processes.is_empty() {
      let Some(event) = self.rx.recv().await else {
        break;
      };

      match event {
        Event::Line(id, line) => {
          if let Some(proc) = self.processes.get(&id) {
            println!("{}", self.compose_line(proc, &line));
          }
        }
        Event::ProcessEnded(id) => self.handle_exit(id).await,
        Event::Restart(id) => self.handle_restart(id),
        Event::CtrlC => self.handle_ctrlc(),
        Event::ForceKill => self.signal_all(Signal::SIGKILL, "Killing..."),
      }
    }

    self.exit_code()
  }

  /// Print a "Spawned, pid: ..." message for a newly started process.
  fn log_spawn(&self, proc: &Process) {
    if let Some(pid) = proc.pid() {
      println!(
        "{}",
        self.compose_system_line(proc, &format!("Spawned, pid: {}", pid))
      );
    }
  }

  /// Format a system message like "[web] Stopping..." with the process prefix.
  fn compose_system_line(&self, proc: &Process, line: &str) -> String {
    self.compose_line(proc, &format!("[{}] {}", proc.name, line))
  }

  /// Format an output line with the colored process name prefix (and optional timestamp).
  fn compose_line(&self, proc: &Process, line: &str) -> String {
    let mut parts = Vec::new();

    if self.timestamps {
      let now = chrono::Local::now();
      parts.push(
        now
          .format(TIMESTAMP_FORMAT)
          .to_string()
          .color(proc.color)
          .to_string(),
      );
    }

    if self.compact {
      parts.push(COMPACT_INDICATOR.color(proc.color).to_string());
    } else {
      let width = self.name_width;
      parts.push(
        format!("{:width$} |", proc.name)
          .color(proc.color)
          .to_string(),
      );
    }

    format!("{} {}", parts.join(" "), line)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn from_string_fails_if_no_processes_selected() {
    let exclude = vec!["web".to_string()];
    let include = Vec::<String>::new();
    let input = "web: echo hi\n";

    let manager =
      ProcessManager::from_string(input, RunningMode::Exit, &exclude, &include, false, false);

    match manager {
      Ok(_) => panic!("expected no-processes error"),
      Err(err) => assert!(err.contains("No processes selected")),
    }
  }
}
