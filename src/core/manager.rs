use super::ansi;
use super::color::color_for_index;
use super::process::Process;
use super::LogEvent;
use crate::input;
use clap::ValueEnum;
use nix::sys::signal::Signal;
use pty_process::Pty;
use std::collections::HashMap;
use std::fmt::Display;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;
use std::time::Duration;
use tokio::io::{BufReader, Lines};
use tokio::sync::{broadcast, mpsc};
use tokio::time::sleep;

/// How long to wait after SIGINT before sending SIGKILL.
const SIGKILL_GRACE_SECS: u64 = 5;

#[derive(Debug, Clone, Copy, PartialOrd, PartialEq, ValueEnum)]
pub enum OnExit {
  /// Restart a process when it exits
  Restart,
  /// Stop all processes when any one exits
  Stop,
  /// Ignore process exits, quit when all are done
  Ignore,
}

impl Display for OnExit {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      OnExit::Restart => write!(f, "Restart"),
      OnExit::Stop => write!(f, "Stop"),
      OnExit::Ignore => write!(f, "Ignore"),
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
  /// An interactive command from the user.
  Command(input::Command),
}

/// Owns all child processes and drives the main event loop.
///
/// Lifecycle: `from_string()` parses the Procfile and builds the manager,
/// then `start()` spawns everything and blocks until all processes are done.
pub struct ProcessManager {
  /// Current on-exit behavior (may switch to Ignore during shutdown).
  mode: OnExit,
  /// Longest process name length, used by stdout to align log prefixes.
  name_width: usize,
  /// Active processes keyed by their index from the Procfile.
  processes: HashMap<usize, Process>,
  /// Sender half of the internal event channel — cloned into background tasks.
  tx: mpsc::UnboundedSender<Event>,
  /// Receiver half — consumed exclusively by the main event loop.
  rx: mpsc::UnboundedReceiver<Event>,
  /// Set when any process exits with a non-zero status.
  failed: bool,
  /// Set once graceful shutdown has been initiated (SIGINT sent).
  shutting_down: bool,
  /// Broadcast channel for log consumers (stdout, web).
  log_tx: broadcast::Sender<LogEvent>,
  /// Next process ID for dynamically added processes.
  next_id: usize,
  /// Optional receiver for input events (interactive mode).
  input_rx: Option<mpsc::UnboundedReceiver<input::InputEvent>>,
  /// Optional watch sender for name_width updates.
  name_width_tx: Option<tokio::sync::watch::Sender<usize>>,
}

impl ProcessManager {
  /// Build a manager from a list of (name, command) pairs. Does not start processes yet.
  pub fn new(
    entries: &[(&str, &str)],
    mode: OnExit,
    log_tx: broadcast::Sender<LogEvent>,
  ) -> Result<Self, String> {
    if entries.is_empty() {
      return Err("No processes to run".to_string());
    }

    let name_width = entries.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
    let (tx, rx) = mpsc::unbounded_channel();

    let processes: HashMap<usize, Process> = entries
      .iter()
      .enumerate()
      .map(|(i, (name, cmd))| (i, Process::new(name, cmd, color_for_index(i))))
      .collect();

    Ok(Self {
      processes,
      name_width,
      mode,
      tx,
      rx,
      failed: false,
      shutting_down: false,
      log_tx,
      next_id: entries.len(),
      input_rx: None,
      name_width_tx: None,
    })
  }

  /// Longest process name length (for stdout alignment).
  pub fn name_width(&self) -> usize {
    self.name_width
  }

  /// Set the input event receiver for interactive mode.
  pub fn set_input_rx(&mut self, rx: mpsc::UnboundedReceiver<input::InputEvent>) {
    self.input_rx = Some(rx);
  }

  /// Create a watch channel for name_width and return the receiver.
  pub fn name_width_watch(&mut self) -> tokio::sync::watch::Receiver<usize> {
    let (tx, rx) = tokio::sync::watch::channel(self.name_width);
    self.name_width_tx = Some(tx);
    rx
  }

  /// Find a process ID by name.
  fn find_by_name(&self, name: &str) -> Option<usize> {
    self.processes.iter().find(|(_, p)| p.name == name).map(|(id, _)| *id)
  }

  /// Emit an error message through the broadcast channel (for interactive mode)
  /// or to stderr (for non-interactive mode).
  fn emit_error(&self, message: &str) {
    if self.name_width_tx.is_some() {
      let _ = self.log_tx.send(LogEvent {
        process: "system".to_string(),
        color: colored::Color::Red,
        line: message.to_string(),
        system: true,
      });
    } else {
      eprintln!("{}", message);
    }
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

  /// Send a log event to all consumers (stdout, web).
  fn emit(&self, proc: &Process, line: &str, system: bool) {
    let _ = self.log_tx.send(LogEvent {
      process: proc.name.clone(),
      color: proc.color,
      line: line.to_string(),
      system,
    });
  }

  /// Send a system message like "[web] Stopping..." to all consumers.
  fn emit_system(&self, proc: &Process, message: &str) {
    self.emit(proc, &format!("[{}] {}", proc.name, message), true);
  }

  fn handle_command(&mut self, cmd: input::Command) {
    match cmd {
      input::Command::Kill(name) => {
        let Some(id) = self.find_by_name(&name) else {
          self.emit_error(&format!("Unknown process: {}", name));
          return;
        };
        if let Some(proc) = self.processes.get(&id) {
          if proc.is_running() {
            proc.signal(Signal::SIGINT);
            self.emit_system(proc, "Killing...");
          } else {
            self.emit_system(proc, "Not running");
          }
        }
      }
      input::Command::Restart(name) => {
        let Some(id) = self.find_by_name(&name) else {
          self.emit_error(&format!("Unknown process: {}", name));
          return;
        };
        let running = if let Some(proc) = self.processes.get_mut(&id) {
          proc.pending_restart = true;
          proc.reset_restart_counter();
          let running = proc.is_running();
          if running {
            proc.signal(Signal::SIGINT);
          } else {
            proc.pending_restart = false;
          }
          running
        } else {
          return;
        };
        if running {
          if let Some(proc) = self.processes.get(&id) {
            self.emit_system(proc, "Restarting...");
          }
        } else {
          if let Err(err) = self.start_one(id) {
            self.emit_error(&format!("Failed to restart {}: {}", name, err));
          }
        }
      }
      input::Command::Add(name, cmd) => {
        if self.find_by_name(&name).is_some() {
          self.emit_error(&format!("Process already exists: {}", name));
          return;
        }
        let id = self.next_id;
        self.next_id += 1;
        let process = Process::new(&name, &cmd, color_for_index(id));
        self.processes.insert(id, process);

        if name.len() > self.name_width {
          self.name_width = name.len();
          if let Some(ref tx) = self.name_width_tx {
            let _ = tx.send(self.name_width);
          }
        }

        if let Err(err) = self.start_one(id) {
          self.emit_error(&format!("Failed to start {}: {}", name, err));
          self.processes.remove(&id);
        }
      }
      input::Command::Remove(name) => {
        let Some(id) = self.find_by_name(&name) else {
          self.emit_error(&format!("Unknown process: {}", name));
          return;
        };
        let running = if let Some(proc) = self.processes.get_mut(&id) {
          proc.removed = true;
          let running = proc.is_running();
          if running {
            proc.signal(Signal::SIGINT);
          }
          running
        } else {
          return;
        };
        if running {
          if let Some(proc) = self.processes.get(&id) {
            self.emit_system(proc, "Removing...");
          }
        } else {
          if let Some(proc) = self.processes.get(&id) {
            self.emit_system(proc, "Removed");
          }
          self.processes.remove(&id);
        }
      }
      input::Command::List => {
        for id in self.sorted_ids() {
          if let Some(proc) = self.processes.get(&id) {
            let status = if let Some(pid) = proc.pid() {
              format!("pid: {}, running", pid)
            } else {
              "stopped".to_string()
            };
            self.emit_system(proc, &status);
          }
        }
      }
      input::Command::Help => {}
    }
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

    if let Some(proc) = self.processes.get(&id)
      && let Some(pid) = proc.pid()
    {
      self.emit_system(proc, &format!("Spawned, pid: {}", pid));
    }

    Ok(())
  }

  /// Start every process. On first failure, stop all already-started ones.
  fn start_all(&mut self) {
    for id in self.sorted_ids() {
      if let Err(err) = self.start_one(id) {
        self.failed = true;

        if let Some(proc) = self.processes.get(&id) {
          self.emit_system(proc, &err);
        } else {
          self.emit_error(&format!("Error starting process {}: {}", id, err));
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

    // Check removed flag — skip all mode logic
    if let Some(proc) = self.processes.get(&id)
      && proc.removed
    {
      self.emit_system(proc, "Removed");
      self.processes.remove(&id);
      return;
    }

    // Check pending_restart flag — bypass mode logic, restart immediately
    let has_pending_restart = self
      .processes
      .get_mut(&id)
      .map(|proc| {
        if proc.pending_restart {
          proc.pending_restart = false;
          true
        } else {
          false
        }
      })
      .unwrap_or(false);
    if has_pending_restart {
      if let Some(proc) = self.processes.get(&id) {
        self.emit_system(proc, &exit_message);
      }
      if let Err(err) = self.start_one(id) {
        self.emit_error(&format!("Failed to restart: {}", err));
        self.processes.remove(&id);
      }
      return;
    }

    match self.mode {
      OnExit::Restart => {
        if let Some(proc) = self.processes.get(&id) {
          self.emit_system(proc, &exit_message);
        }

        // Process stays in the map (child=None) while waiting for the restart timer.
        // If shutdown happens in between, stop() removes childless entries via retain().
        let delay: Option<Duration> = self
          .processes
          .get_mut(&id)
          .map(|proc| proc.next_restart_delay());

        if let Some(delay) = delay {
          if let Some(proc) = self.processes.get(&id) {
            self.emit_system(proc, &format!("Restarting in {}ms...", delay.as_millis()));
          }

          Self::spawn_restart(id, delay, self.tx.clone());
        }
      }
      OnExit::Stop => {
        if !exit_success {
          self.failed = true;
        }

        if let Some(proc) = self.processes.get(&id) {
          self.emit_system(proc, &exit_message);
        }

        self.processes.remove(&id);
        self.stop();
      }
      OnExit::Ignore => {
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
          self.emit_system(proc, message);
        }

        self.processes.remove(&id);
      }
    }
  }

  /// Called when a restart timer fires. Starts the process again (or drops it on failure).
  fn handle_restart(&mut self, id: usize) {
    if let Some(proc) = self.processes.get(&id)
      && proc.removed
    {
      self.processes.remove(&id);
      return;
    }

    if self.mode != OnExit::Restart || self.shutting_down {
      self.processes.remove(&id);
      return;
    }

    if let Err(err) = self.start_one(id) {
      self.failed = true;

      if let Some(proc) = self.processes.get(&id) {
        self.emit_system(proc, &err);
      } else {
        self.emit_error(&format!("Error restarting process {}: {}", id, err));
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
        self.emit_system(process, message);
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
    self.mode = OnExit::Ignore;
    self.processes.retain(|_, process| process.is_running());
    self.signal_all(Signal::SIGINT, "Stopping...");
    Self::spawn_force_kill(self.tx.clone());
  }

  /// First Ctrl+C starts graceful shutdown; second one sends SIGKILL immediately.
  fn handle_ctrlc(&mut self) {
    if !self.shutting_down {
      self.emit_error("Ctrl+C received, stopping processes...");
      self.stop();
    } else {
      self.emit_error("Ctrl+C received again, killing processes...");
      self.signal_all(Signal::SIGKILL, "Killing...");
    }
  }

  /// 0 if all processes exited cleanly, 1 if any failed.
  fn exit_code(&self) -> u8 {
    if self.failed { 1 } else { 0 }
  }

  /// Main event loop. Spawns all processes, then dispatches events
  /// until every process has exited and been removed from the map.
  pub async fn start(&mut self, interactive: bool) -> u8 {
    if !interactive {
      Self::spawn_ctrlc(self.tx.clone());
    }
    self.start_all();

    while !self.processes.is_empty() {
      let event = if let Some(ref mut input_rx) = self.input_rx {
        tokio::select! {
          ev = self.rx.recv() => {
            match ev {
              Some(e) => e,
              None => break,
            }
          }
          ev = input_rx.recv() => {
            match ev {
              Some(input::InputEvent::Command(cmd)) => Event::Command(cmd),
              Some(input::InputEvent::CtrlC) => Event::CtrlC,
              None => continue,
            }
          }
        }
      } else {
        match self.rx.recv().await {
          Some(e) => e,
          None => break,
        }
      };

      match event {
        Event::Line(id, line) => {
          if let Some(proc) = self.processes.get(&id) {
            self.emit(proc, &line, false);
          }
        }
        Event::ProcessEnded(id) => self.handle_exit(id).await,
        Event::Restart(id) => self.handle_restart(id),
        Event::CtrlC => self.handle_ctrlc(),
        Event::ForceKill => self.signal_all(Signal::SIGKILL, "Killing..."),
        Event::Command(cmd) => self.handle_command(cmd),
      }
    }

    self.exit_code()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn new_fails_if_no_processes() {
    let (log_tx, _) = broadcast::channel(16);
    let manager = ProcessManager::new(&[], OnExit::Stop, log_tx);

    match manager {
      Ok(_) => panic!("expected no-processes error"),
      Err(err) => assert!(err.contains("No processes")),
    }
  }
}
