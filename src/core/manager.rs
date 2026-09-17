use super::LogEvent;
use super::ansi;
use super::color::color_for_index;
use super::process::Process;
use super::procfile::{Flags, ProcessSpec};
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

impl OnExit {
  /// How this policy reads when it is pinned to a single process rather than
  /// the whole session — `Ignore` on one process is what a Procfile calls
  /// `once`.
  pub fn process_label(self) -> &'static str {
    match self {
      OnExit::Restart => "restart",
      OnExit::Stop => "stop the run",
      OnExit::Ignore => "once",
    }
  }
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
  /// The process's current flags: whether it was declared `optional`, and its
  /// own exit policy when it has one.
  pub flags: Flags,
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
  /// `delay:<dur>` elapsed — time to make process `id`'s first start.
  DelayedStart(usize),
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
  /// Broadcast channel for log consumers.
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

/// How a process's own exit policy reads next to the session's.
pub fn describe_mode(mode: Option<OnExit>, session: OnExit) -> String {
  match mode {
    Some(mode) => mode.process_label().to_string(),
    None => format!("follow session ({})", session),
  }
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
  /// Build a manager from a list of process definitions. Does not start
  /// processes yet.
  pub fn new(
    entries: &[ProcessSpec],
    mode: OnExit,
    log_tx: broadcast::Sender<LogEvent>,
  ) -> Result<Self, String> {
    if entries.is_empty() {
      return Err("No processes to run".to_string());
    }

    let name_width = entries
      .iter()
      .map(|spec| spec.name.len())
      .max()
      .unwrap_or(0);
    let (tx, rx) = mpsc::unbounded_channel();

    let processes: HashMap<usize, Process> = entries
      .iter()
      .enumerate()
      .map(|(i, spec)| {
        (
          i,
          Process::new(&spec.name, &spec.cmd, color_for_index(i), spec.flags),
        )
      })
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

  /// Bring `optional` processes into the run before `start()`, as asked for
  /// on the command line. Unknown names are returned rather than reported, so
  /// the caller can decide whether they are a mistake.
  pub fn enable(&mut self, names: &[String]) -> Vec<String> {
    let mut unknown = Vec::new();
    for name in names {
      match self.find_by_name(name) {
        Some(id) => {
          if let Some(proc) = self.processes.get_mut(&id) {
            proc.enabled = true;
          }
        }
        None => unknown.push(name.clone()),
      }
    }
    unknown
  }

  /// Bring every `optional` process into the run.
  pub fn enable_all(&mut self) {
    for proc in self.processes.values_mut() {
      proc.enabled = true;
    }
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
          flags: proc.flags(),
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

  /// Schedule the deferred first start of a `delay:<dur>` process.
  fn spawn_delayed_start(id: usize, delay: Duration, tx: mpsc::UnboundedSender<Event>) {
    tokio::spawn(async move {
      sleep(delay).await;
      let _ = tx.send(Event::DelayedStart(id));
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

  /// Send a log event to all consumers.
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
      // A bulk action is about the processes in the run. An optional one that
      // nobody has enabled yet is not in it — reaching it takes naming it.
      input::Target::All => {
        return Some(
          self
            .sorted_ids()
            .into_iter()
            .filter(|id| !self.processes.get(id).is_some_and(Process::is_dormant))
            .collect(),
        );
      }
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
      input::Command::ProcessMode(target, mode) => {
        let Some(ids) = self.resolve(&target) else {
          return;
        };
        for id in ids {
          self.set_process_mode(id, mode);
        }
      }
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

    // Starting an optional process by name is how it joins the run.
    if let Some(proc) = self.processes.get_mut(&id) {
      proc.stopped = false;
      proc.enabled = true;
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
    proc.enabled = true;
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

  /// Pin a process to its own exit policy, or hand it back to the session
  /// mode with `None`.
  fn set_process_mode(&mut self, id: usize, mode: Option<OnExit>) {
    let Some(proc) = self.processes.get_mut(&id) else {
      return;
    };
    proc.on_exit = mode;

    let (name, color) = (proc.name.clone(), proc.color);
    let session = self.mode;
    self.emit_reply(
      &name,
      color,
      format!("[{}] Run mode: {}", name, describe_mode(mode, session)),
    );
  }

  /// Add a process at runtime. The name may carry Procfile flags, so
  /// `migrate(once)` works here exactly as it does in the file.
  fn add_process(&mut self, name: &str, cmd: &str) {
    let (name, flags) = match crate::core::procfile::parse_name(name) {
      Ok(parsed) => parsed,
      Err(reason) => {
        self.emit_error(&format!("Process {:?} {}", name, reason));
        return;
      }
    };

    if name.is_empty() {
      self.emit_error("Process doesn't have a name");
      return;
    }

    if self.find_by_name(name).is_some() {
      self.emit_error(&format!("Process already exists: {}", name));
      return;
    }

    let id = self.next_id;
    self.next_id += 1;
    self
      .processes
      .insert(id, Process::new(name, cmd, color_for_index(id), flags));

    if name.len() > self.name_width {
      self.name_width = name.len();
      if let Some(ref tx) = self.name_width_tx {
        let _ = tx.send(self.name_width);
      }
    }

    // `add worker(optional)` registers the process without running it, the
    // same as an optional line in the Procfile.
    if flags.optional {
      if let Some(proc) = self.processes.get(&id) {
        self.emit_system(proc, "Optional — not started");
      }
      return;
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
    } else if proc.is_dormant() {
      "optional"
    } else if proc.stopped {
      "stopped"
    } else {
      "restarting"
    }
  }

  /// The exit policy that actually applies to one process: its own Procfile
  /// flag when it has one, otherwise the session mode.
  ///
  /// Shutdown outranks both — once SIGINT has gone out, a `restart` flag must
  /// not keep resurrecting the process the session is trying to wind down.
  fn effective_on_exit(&self, id: usize) -> OnExit {
    if self.shutting_down {
      return OnExit::Ignore;
    }
    self
      .processes
      .get(&id)
      .and_then(|proc| proc.on_exit)
      .unwrap_or(self.mode)
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
      if self.processes.get(&id).is_some_and(Process::is_dormant) {
        if let Some(proc) = self.processes.get(&id) {
          self.emit_system(proc, "Optional — not started");
        }
        continue;
      }

      // `delay` only holds back the automatic start: the process stays in the
      // table meanwhile, and an explicit start still takes effect at once.
      if let Some(delay) = self.processes.get(&id).and_then(|proc| proc.delay) {
        if let Some(proc) = self.processes.get(&id) {
          self.emit_system(proc, &format!("Starting in {}ms...", delay.as_millis()));
        }
        Self::spawn_delayed_start(id, delay, self.tx.clone());
        continue;
      }

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

    // `allow-failure` keeps this process's exit status out of the run's own:
    // a linter or a smoke check can fail without failing the session.
    let counts_as_failure = !exit_success
      && !self
        .processes
        .get(&id)
        .is_some_and(|proc| proc.allow_failure);

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

    match self.effective_on_exit(id) {
      OnExit::Restart => {
        if let Some(proc) = self.processes.get(&id) {
          self.emit_system(proc, &exit_message);
        }

        // `retries:<n>` caps the loop: a process that cannot stay up should
        // stop burning the terminal down rather than respawn forever.
        if self
          .processes
          .get(&id)
          .is_some_and(Process::retries_exhausted)
        {
          if counts_as_failure {
            self.failed = true;
          }

          let attempts = self
            .processes
            .get(&id)
            .and_then(|proc| proc.retries)
            .unwrap_or(0);

          if let Some(proc) = self.processes.get(&id) {
            self.emit_system(
              proc,
              &format!("Giving up — restart limit reached (retries:{})", attempts),
            );
          }

          // Interactively the row stays so it can be inspected and started
          // again by hand; headless there is nobody to do that.
          if self.is_interactive() {
            if let Some(proc) = self.processes.get_mut(&id) {
              proc.stopped = true;
            }
          } else {
            self.processes.remove(&id);
          }
          return;
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
        if counts_as_failure {
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
        if counts_as_failure && !self.shutting_down {
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

  /// Called when a `delay:<dur>` timer fires. Anything that happened during
  /// the wait — a stop, a remove, an explicit start, a shutdown — outranks it.
  fn handle_delayed_start(&mut self, id: usize) {
    let Some(proc) = self.processes.get(&id) else {
      return;
    };

    if self.shutting_down || proc.stopped || proc.is_running() || proc.is_dormant() {
      return;
    }

    if let Err(err) = self.start_one(id) {
      self.failed = true;

      if let Some(proc) = self.processes.get(&id) {
        self.emit_system(proc, &err);
      }

      self.processes.remove(&id);
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

    if self.effective_on_exit(id) != OnExit::Restart {
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
        Event::DelayedStart(id) => self.handle_delayed_start(id),
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

  fn spec(name: &str, flags: Flags) -> ProcessSpec {
    ProcessSpec {
      name: name.to_string(),
      cmd: "echo hi".to_string(),
      flags,
    }
  }

  fn optional() -> Flags {
    Flags {
      optional: true,
      ..Flags::default()
    }
  }

  fn manager(specs: &[ProcessSpec], mode: OnExit) -> ProcessManager {
    let (log_tx, _) = broadcast::channel(16);
    ProcessManager::new(specs, mode, log_tx).expect("specs are non-empty")
  }

  fn status(manager: &ProcessManager, name: &str) -> &'static str {
    let id = manager.find_by_name(name).expect("process exists");
    ProcessManager::status_of(&manager.processes[&id])
  }

  fn is_dormant(manager: &ProcessManager, name: &str) -> bool {
    let id = manager.find_by_name(name).expect("process exists");
    manager.processes[&id].is_dormant()
  }

  #[test]
  fn new_fails_if_no_processes() {
    let (log_tx, _) = broadcast::channel(16);
    let manager = ProcessManager::new(&[], OnExit::Stop, log_tx);

    match manager {
      Ok(_) => panic!("expected no-processes error"),
      Err(err) => assert!(err.contains("No processes")),
    }
  }

  #[test]
  fn a_process_flag_outranks_the_session_mode() {
    let manager = manager(
      &[
        spec("web", Flags::default()),
        spec(
          "migrate",
          Flags {
            on_exit: Some(OnExit::Ignore),
            ..Flags::default()
          },
        ),
      ],
      OnExit::Stop,
    );

    let web = manager.find_by_name("web").unwrap();
    let migrate = manager.find_by_name("migrate").unwrap();

    assert_eq!(manager.effective_on_exit(web), OnExit::Stop);
    assert_eq!(manager.effective_on_exit(migrate), OnExit::Ignore);
  }

  #[test]
  fn shutdown_outranks_a_restart_flag() {
    let mut manager = manager(
      &[spec(
        "web",
        Flags {
          on_exit: Some(OnExit::Restart),
          ..Flags::default()
        },
      )],
      OnExit::Ignore,
    );
    let web = manager.find_by_name("web").unwrap();
    assert_eq!(manager.effective_on_exit(web), OnExit::Restart);

    manager.shutting_down = true;
    assert_eq!(manager.effective_on_exit(web), OnExit::Ignore);
  }

  #[test]
  fn optional_processes_stay_out_of_the_run_until_enabled() {
    let mut manager = manager(
      &[spec("web", Flags::default()), spec("seed", optional())],
      OnExit::Stop,
    );

    assert_eq!(status(&manager, "seed"), "optional");
    assert!(is_dormant(&manager, "seed"));
    assert!(!is_dormant(&manager, "web"));

    assert!(manager.enable(&["seed".to_string()]).is_empty());
    assert!(!is_dormant(&manager, "seed"));
  }

  #[test]
  fn enable_reports_names_it_does_not_know() {
    let mut manager = manager(&[spec("web", Flags::default())], OnExit::Stop);
    assert_eq!(manager.enable(&["nope".to_string()]), vec!["nope"]);
  }

  #[test]
  fn enable_all_brings_every_optional_process_in() {
    let mut manager = manager(
      &[spec("seed", optional()), spec("backfill", optional())],
      OnExit::Stop,
    );

    manager.enable_all();

    assert!(!is_dormant(&manager, "seed"));
    assert!(!is_dormant(&manager, "backfill"));
  }

  #[test]
  fn bulk_targets_skip_optional_processes_but_names_reach_them() {
    let mut manager = manager(
      &[spec("web", Flags::default()), spec("seed", optional())],
      OnExit::Stop,
    );
    let web = manager.find_by_name("web").unwrap();
    let seed = manager.find_by_name("seed").unwrap();

    assert_eq!(manager.resolve(&input::Target::All), Some(vec![web]));
    assert_eq!(
      manager.resolve(&input::Target::Names(vec!["seed".to_string()])),
      Some(vec![seed])
    );

    // Once enabled it joins the run, and bulk actions pick it up.
    manager.enable(&["seed".to_string()]);
    let mut all = manager.resolve(&input::Target::All).unwrap();
    all.sort_unstable();
    assert_eq!(all, vec![web, seed].into_iter().collect::<Vec<_>>());
  }

  #[test]
  fn setting_a_process_mode_leaves_the_session_and_its_peers_alone() {
    let mut manager = manager(
      &[
        spec("web", Flags::default()),
        spec("worker", Flags::default()),
      ],
      OnExit::Stop,
    );
    let web = manager.find_by_name("web").unwrap();
    let worker = manager.find_by_name("worker").unwrap();

    manager.set_process_mode(web, Some(OnExit::Restart));

    assert_eq!(manager.mode, OnExit::Stop);
    assert_eq!(manager.effective_on_exit(web), OnExit::Restart);
    assert_eq!(manager.effective_on_exit(worker), OnExit::Stop);

    // Handing it back means following the session again.
    manager.set_process_mode(web, None);
    assert_eq!(manager.effective_on_exit(web), OnExit::Stop);
  }

  #[test]
  fn added_processes_can_carry_flags_in_their_name() {
    let mut manager = manager(&[spec("web", Flags::default())], OnExit::Stop);

    manager.add_process("seed(optional, once)", "echo seed");

    let seed = manager.find_by_name("seed").expect("seed was added");
    assert_eq!(
      manager.processes[&seed].flags(),
      Flags {
        optional: true,
        on_exit: Some(OnExit::Ignore),
        ..Flags::default()
      }
    );
    // Optional, so adding it registered the process without spawning it.
    assert_eq!(status(&manager, "seed"), "optional");
  }

  #[test]
  fn a_retry_ceiling_is_measured_against_attempts_already_made() {
    let flags = Flags {
      on_exit: Some(OnExit::Restart),
      retries: Some(2),
      ..Flags::default()
    };
    let mut manager = manager(&[spec("web", flags)], OnExit::Ignore);
    let web = manager.find_by_name("web").unwrap();
    let proc = manager.processes.get_mut(&web).unwrap();

    assert!(!proc.retries_exhausted());
    proc.next_restart_delay();
    assert!(!proc.retries_exhausted());
    proc.next_restart_delay();
    assert!(proc.retries_exhausted());

    // An explicit start or restart clears the counter, so the ceiling is on
    // consecutive automatic attempts rather than the life of the session.
    proc.reset_restart_counter();
    assert!(!proc.retries_exhausted());
  }

  #[test]
  fn retries_of_zero_gives_up_on_the_very_first_exit() {
    let flags = Flags {
      on_exit: Some(OnExit::Restart),
      retries: Some(0),
      ..Flags::default()
    };
    let manager = manager(&[spec("web", flags)], OnExit::Ignore);
    let web = manager.find_by_name("web").unwrap();

    assert!(manager.processes[&web].retries_exhausted());
  }

  #[test]
  fn no_retry_flag_means_no_ceiling() {
    let flags = Flags {
      on_exit: Some(OnExit::Restart),
      ..Flags::default()
    };
    let mut manager = manager(&[spec("web", flags)], OnExit::Ignore);
    let web = manager.find_by_name("web").unwrap();
    let proc = manager.processes.get_mut(&web).unwrap();

    for _ in 0..100 {
      proc.next_restart_delay();
      assert!(!proc.retries_exhausted());
    }
  }

  #[test]
  fn flags_survive_the_trip_through_a_snapshot() {
    let flags = Flags {
      optional: true,
      on_exit: Some(OnExit::Restart),
      muted: true,
      allow_failure: true,
      delay: Some(Duration::from_millis(1500)),
      retries: Some(3),
    };
    let manager = manager(&[spec("web", flags)], OnExit::Stop);

    let snapshot = manager.snapshot();
    assert_eq!(snapshot.processes[0].flags, flags);
    assert_eq!(
      snapshot.processes[0].flags.suffix(),
      "(optional, restart, delay:1500ms, retries:3, muted, allow-failure)"
    );
  }

  #[test]
  fn a_delayed_start_is_dropped_once_the_session_is_winding_down() {
    let flags = Flags {
      delay: Some(Duration::from_secs(5)),
      ..Flags::default()
    };
    let mut manager = manager(&[spec("web", flags)], OnExit::Ignore);
    let web = manager.find_by_name("web").unwrap();

    // A timer that fires after shutdown began must not spawn anything.
    manager.shutting_down = true;
    manager.handle_delayed_start(web);
    assert!(!manager.processes[&web].is_running());

    // Nor must one whose process was stopped by hand during the wait.
    manager.shutting_down = false;
    manager.processes.get_mut(&web).unwrap().stopped = true;
    manager.handle_delayed_start(web);
    assert!(!manager.processes[&web].is_running());
  }

  #[test]
  fn adding_a_process_with_a_bad_flag_does_not_register_it() {
    let mut manager = manager(&[spec("web", Flags::default())], OnExit::Stop);

    manager.add_process("seed(nope)", "echo seed");

    assert!(manager.find_by_name("seed").is_none());
  }
}
