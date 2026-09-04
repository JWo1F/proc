use super::LogEvent;
use super::ansi;
use super::color::color_for_index;
use super::process::Process;
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

/// A point-in-time view of one process, for rendering in the interactive modal.
#[derive(Clone, Debug)]
pub struct ProcessSnapshot {
  pub name: String,
  pub command: String,
  pub status: &'static str,
  pub pid: Option<u32>,
  pub uptime: Option<Duration>,
  pub restarts: u32,
  pub last_exit: Option<String>,
}

/// A point-in-time view of the whole session, published after every event so
/// the modal can render live without going through the log broadcast.
#[derive(Clone, Debug)]
pub struct Snapshot {
  pub mode: OnExit,
  pub processes: Vec<ProcessSnapshot>,
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
  /// Optional watch sender publishing a full session snapshot for the modal.
  snapshot_tx: Option<tokio::sync::watch::Sender<Snapshot>>,
}

/// Render a duration as a compact, fixed-shape uptime string.
pub fn format_uptime(duration: Duration) -> String {
  let seconds = duration.as_secs();
  if seconds < 60 {
    format!("{}s", seconds)
  } else if seconds < 3600 {
    format!("{}m{:02}s", seconds / 60, seconds % 60)
  } else {
    format!("{}h{:02}m", seconds / 3600, (seconds % 3600) / 60)
  }
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

    let name_width = entries
      .iter()
      .map(|(name, _)| name.len())
      .max()
      .unwrap_or(0);
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
      snapshot_tx: None,
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

  /// Create a watch channel publishing a full session snapshot, for the modal.
  pub fn snapshot_watch(&mut self) -> tokio::sync::watch::Receiver<Snapshot> {
    let (tx, rx) = tokio::sync::watch::channel(self.snapshot());
    self.snapshot_tx = Some(tx);
    rx
  }

  /// Build a fresh snapshot of the current session state.
  fn snapshot(&self) -> Snapshot {
    Snapshot {
      mode: self.mode,
      processes: self
        .sorted_ids()
        .iter()
        .filter_map(|id| self.processes.get(id))
        .map(|proc| ProcessSnapshot {
          name: proc.name.clone(),
          command: proc.cmd.clone(),
          status: Self::status_of(proc),
          pid: proc.pid(),
          uptime: proc.uptime(),
          restarts: proc.restarts(),
          last_exit: proc.last_exit.clone(),
        })
        .collect(),
    }
  }

  /// Republish the snapshot after anything that could have changed it.
  fn publish_snapshot(&self) {
    if let Some(ref tx) = self.snapshot_tx {
      let _ = tx.send(self.snapshot());
    }
  }

  /// Find a process ID by name.
  fn find_by_name(&self, name: &str) -> Option<usize> {
    self
      .processes
      .iter()
      .find(|(_, p)| p.name == name)
      .map(|(id, _)| *id)
  }

  /// Whether an interactive prompt is attached.
  fn is_interactive(&self) -> bool {
    self.input_rx.is_some()
  }

  /// Emit an error message through the broadcast channel (for interactive mode)
  /// or to stderr (for non-interactive mode).
  fn emit_error(&self, message: &str) {
    if self.is_interactive() {
      let _ = self.log_tx.send(LogEvent {
        process: "system".to_string(),
        color: colored::Color::Red,
        line: message.to_string(),
        system: true,
        reply: true,
      });
    } else {
      eprintln!("{}", message);
    }
  }

  /// Emit a direct answer to an interactive command. Carries the process's own
  /// name and color so the log gutter keeps identifying who a line is about.
  fn emit_reply(&self, process: &str, color: colored::Color, line: String) {
    let _ = self.log_tx.send(LogEvent {
      process: process.to_string(),
      color,
      line,
      system: true,
      reply: true,
    });
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
      reply: false,
    });
  }

  /// Send a system message like "[web] Stopping..." to all consumers.
  fn emit_system(&self, proc: &Process, message: &str) {
    self.emit(proc, &format!("[{}] {}", proc.name, message), true);
  }

  /// Resolve a target list to process IDs, reporting every unknown name at once
  /// rather than failing on the first one.
  fn resolve(&self, target: &input::Target) -> Option<Vec<usize>> {
    let names = match target {
      input::Target::All => return Some(self.sorted_ids()),
      input::Target::Names(names) => names,
    };

    let mut ids = Vec::new();
    let mut unknown = Vec::new();
    for name in names {
      match self.find_by_name(name) {
        Some(id) => ids.push(id),
        None => unknown.push(name.clone()),
      }
    }

    if !unknown.is_empty() {
      self.emit_error(&format!("Unknown process: {}", unknown.join(", ")));
      return None;
    }

    ids.sort_unstable();
    ids.dedup();
    Some(ids)
  }

  fn handle_command(&mut self, cmd: input::Command) {
    match cmd {
      input::Command::Start(target) => {
        let Some(ids) = self.resolve(&target) else {
          return;
        };
        for id in ids {
          self.start_target(id);
        }
      }
      input::Command::Stop(target) => {
        let Some(ids) = self.resolve(&target) else {
          return;
        };
        for id in ids {
          self.signal_target(id, Signal::SIGINT, "Stopping...");
        }
      }
      input::Command::Kill(target) => {
        let Some(ids) = self.resolve(&target) else {
          return;
        };
        for id in ids {
          self.signal_target(id, Signal::SIGKILL, "Killing...");
        }
      }
      input::Command::Restart(target) => {
        let Some(ids) = self.resolve(&target) else {
          return;
        };
        for id in ids {
          self.restart_target(id);
        }
      }
      input::Command::Remove(target) => {
        let Some(ids) = self.resolve(&target) else {
          return;
        };
        for id in ids {
          self.remove_target(id);
        }
      }
      input::Command::Add(name, cmd) => self.add_process(&name, &cmd),
      input::Command::Mode(mode) => {
        if self.shutting_down {
          self.emit_error("Shutting down — the on-exit mode can no longer change");
          return;
        }
        self.mode = mode;
        self.emit_reply(
          "system",
          colored::Color::White,
          format!("On-exit mode: {}", mode),
        );
      }
      input::Command::Quit => {
        self.emit_reply(
          "system",
          colored::Color::White,
          "Stopping all processes...".to_string(),
        );
        self.stop();
      }
    }
  }

  /// Start a stopped process, clearing any explicit stop that was holding it down.
  fn start_target(&mut self, id: usize) {
    if self.processes.get(&id).is_some_and(|p| p.is_running()) {
      if let Some(proc) = self.processes.get(&id) {
        self.emit_system(proc, "Already running");
      }
      return;
    }

    if let Some(proc) = self.processes.get_mut(&id) {
      proc.stopped = false;
      proc.reset_restart_counter();
    }

    if let Err(err) = self.start_one(id) {
      self.emit_error(&format!("Failed to start: {}", err));
    }
  }

  /// Signal a process and mark it explicitly stopped, so the `--on-exit`
  /// policy does not immediately undo what the user just asked for.
  fn signal_target(&mut self, id: usize, signal: Signal, message: &str) {
    let running = self.processes.get(&id).is_some_and(|p| p.is_running());

    if let Some(proc) = self.processes.get_mut(&id) {
      proc.stopped = true;
    }

    let Some(proc) = self.processes.get(&id) else {
      return;
    };

    if !running {
      self.emit_system(proc, "Already stopped");
      return;
    }

    if let Err(err) = proc.signal(signal) {
      self.emit_error(&err);
      return;
    }
    self.emit_system(proc, message);
  }

  fn restart_target(&mut self, id: usize) {
    let Some(proc) = self.processes.get_mut(&id) else {
      return;
    };

    proc.stopped = false;
    proc.reset_restart_counter();
    let running = proc.is_running();
    proc.pending_restart = running;

    if !running {
      if let Err(err) = self.start_one(id) {
        self.emit_error(&format!("Failed to restart: {}", err));
      }
      return;
    }

    let Some(proc) = self.processes.get(&id) else {
      return;
    };
    if let Err(err) = proc.signal(Signal::SIGINT) {
      self.emit_error(&err);
      return;
    }
    self.emit_system(proc, "Restarting...");
  }

  /// Drop a process from the table. A running child is stopped gracefully first
  /// and the entry disappears once it actually exits.
  fn remove_target(&mut self, id: usize) {
    let running = self.processes.get(&id).is_some_and(|p| p.is_running());

    if let Some(proc) = self.processes.get_mut(&id) {
      proc.stopped = true;
      proc.pending_remove = true;
    }

    if running {
      if let Some(proc) = self.processes.get(&id) {
        if let Err(err) = proc.signal(Signal::SIGINT) {
          self.emit_error(&err);
        }
        self.emit_system(proc, "Removing...");
      }
      return;
    }

    if let Some(proc) = self.processes.get(&id) {
      self.emit_system(proc, "Removed");
    }
    self.processes.remove(&id);
  }

  fn add_process(&mut self, name: &str, cmd: &str) {
    if self.find_by_name(name).is_some() {
      self.emit_error(&format!("Process already exists: {}", name));
      return;
    }

    let id = self.next_id;
    self.next_id += 1;
    self
      .processes
      .insert(id, Process::new(name, cmd, color_for_index(id)));

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

  /// One-word status for the process table.
  fn status_of(proc: &Process) -> &'static str {
    if proc.is_running() {
      "running"
    } else if proc.stopped {
      "stopped"
    } else {
      "restarting"
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

    if let Some(proc) = self.processes.get_mut(&id) {
      proc.last_exit = Some(exit_message.clone());
    }

    // An explicit `stop`/`kill`/`remove` outranks the on-exit policy: skip all
    // mode logic so the process stays down until the user says otherwise.
    let (stopped, pending_remove) = self
      .processes
      .get(&id)
      .map_or((false, false), |proc| (proc.stopped, proc.pending_remove));

    if stopped {
      if let Some(proc) = self.processes.get(&id) {
        self.emit_system(proc, if pending_remove { "Removed" } else { "Stopped" });
      }
      if pending_remove {
        self.processes.remove(&id);
      }
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

        // Interactively the table is the interface, so keep the row: `ps` still
        // lists the process and `start` can bring it back. Headless there is no
        // one to revive it, and the run only ends once the table empties.
        if self.is_interactive() && !self.shutting_down {
          if let Some(proc) = self.processes.get_mut(&id) {
            proc.stopped = true;
          }
        } else {
          self.processes.remove(&id);
        }
      }
    }
  }

  /// Called when a restart timer fires. Starts the process again (or drops it on failure).
  fn handle_restart(&mut self, id: usize) {
    if let Some(proc) = self.processes.get(&id)
      && proc.stopped
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
        if let Err(err) = process.signal(signal) {
          self.emit_error(&err);
        }
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

    // In interactive mode the session outlives the process set: an empty table
    // is a prompt you can still `add` to, not a reason to exit.
    while !self.processes.is_empty() || (interactive && !self.shutting_down) {
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

      self.publish_snapshot();
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
