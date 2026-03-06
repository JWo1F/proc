use crate::ansi;
use crate::process::Process;
use crate::signal::ChildSignal;
use clap::ValueEnum;
use colored::{Color, Colorize};
use nix::sys::signal::Signal;
use pty_process::Pty;
use std::collections::HashMap;
use std::fmt::Display;
use tokio::io::{BufReader, Lines};
use tokio::sync::mpsc;

const COMMENT_PREFIX: char = '#';
const DISABLED_PREFIX: char = '_';
const NAME_CMD_SEPARATOR: &str = ":";
const TIMESTAMP_FORMAT: &str = "%H:%M:%S";
const COMPACT_INDICATOR: &str = "▌";

#[derive(Debug, Clone, PartialOrd, PartialEq, ValueEnum)]
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

enum Event {
  Line(usize, String),
  EOF(usize),
  CtrlC,
}

pub struct ProcessManager {
  mode: RunningMode,
  name_width: usize,
  processes: HashMap<usize, Process>,
  timestamps: bool,
  compact: bool,
  tx: mpsc::UnboundedSender<Event>,
  rx: mpsc::UnboundedReceiver<Event>,
}

static COLORS: [Color; 8] = [
  Color::Red,
  Color::Green,
  Color::Blue,
  Color::Cyan,
  Color::Magenta,
  Color::Yellow,
  Color::White,
  Color::BrightBlack,
];

impl ProcessManager {
  pub fn from_string(
    input: &str,
    mode: RunningMode,
    exclude: &[String],
    include: &[String],
    timestamps: bool,
    compact: bool,
  ) -> Result<Self, String> {
    let parsed = Self::parse_lines(input, exclude, include)?;
    let name_width = parsed.iter().map(|(name, _)| name.len()).max().unwrap_or(0);

    let (tx, rx) = mpsc::unbounded_channel();

    let processes: HashMap<usize, Process> = parsed
      .iter()
      .enumerate()
      .map(|(i, (name, cmd))| (i, name, cmd, COLORS[i % COLORS.len()]))
      .map(|(i, name, cmd, color)| (i, Process::new(name, cmd, color)))
      .collect();

    let mut manager = Self {
      processes,
      name_width,
      mode,
      timestamps,
      compact,
      tx,
      rx,
    };

    for (&id, proc) in manager.processes.iter_mut() {
      let reader = proc.start()?;
      Self::spawn_reader(id, reader, manager.tx.clone());
    }

    for proc in manager.processes.values() {
      manager.log_spawn(proc);
    }

    Self::spawn_ctrlc(manager.tx.clone());

    Ok(manager)
  }

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
            let _ = tx.send(Event::EOF(id));
            break;
          }
          Err(err) => {
            eprintln!("Error reading process output: {}", err);
            let _ = tx.send(Event::EOF(id));
            break;
          }
        }
      }
    });
  }

  fn spawn_ctrlc(tx: mpsc::UnboundedSender<Event>) {
    tokio::spawn(async move {
      let _ = tokio::signal::ctrl_c().await;
      let _ = tx.send(Event::CtrlC);
    });
  }

  fn parse_lines<'a>(
    input: &'a str,
    exclude: &[String],
    include: &[String],
  ) -> Result<Vec<(&'a str, &'a str)>, String> {
    let mut result = Vec::new();
    let mut errors = Vec::new();

    for (n, line) in input.lines().enumerate() {
      if line.starts_with(COMMENT_PREFIX) || line.trim().is_empty() {
        continue;
      }

      let n = n + 1;

      match line.split_once(NAME_CMD_SEPARATOR) {
        Some((name, cmd)) => {
          let name = name.trim();
          let cmd = cmd.trim();

          if name.is_empty() {
            errors.push(format!("Line {} doesn't have a name:\n> {}", n, line));
            continue;
          }

          if cmd.is_empty() {
            errors.push(format!("Line {} doesn't have a command:\n> {}", n, line));
            continue;
          }

          let (name, disabled) = match name.strip_prefix(DISABLED_PREFIX) {
            Some(stripped) => (stripped, true),
            None => (name, false),
          };

          if disabled && !include.iter().any(|i| i == name) {
            continue;
          }

          if exclude.iter().any(|e| e == name) {
            continue;
          }

          result.push((name, cmd));
        }

        None => {
          errors.push(format!(
            "Line {} should contain name and command:\n> {}",
            n, line
          ));
        }
      }
    }

    if errors.is_empty() {
      Ok(result)
    } else {
      Err(errors.join("\n"))
    }
  }

  fn handle_exit(&mut self, id: usize) {
    let Some(proc) = self.processes.get(&id) else {
      return;
    };

    match self.mode {
      RunningMode::Restart => {
        println!("{}", self.compose_system_line(proc, "Restarting..."));

        let Some(proc) = self.processes.get_mut(&id) else {
          return;
        };

        match proc.start() {
          Ok(reader) => {
            self.log_spawn(&self.processes[&id]);
            Self::spawn_reader(id, reader, self.tx.clone());
          }
          Err(err) => {
            eprintln!("{}", self.compose_system_line(&self.processes[&id], &err));
            self.processes.remove(&id);
          }
        }
      }

      RunningMode::Exit => {
        println!("{}", self.compose_system_line(proc, "Exited"));
        self.processes.remove(&id);
        self.stop();
      }

      RunningMode::Relax => {
        println!("{}", self.compose_system_line(proc, "Stopped"));
        self.processes.remove(&id);
      }
    }
  }

  pub fn stop(&mut self) {
    self.mode = RunningMode::Relax;

    for process in self.processes.values() {
      if let Some(child) = &process.child {
        child.signal(Signal::SIGINT);
        println!("{}", self.compose_system_line(process, "Stopping..."));
      }
    }
  }

  pub async fn start(&mut self) {
    while let Some(event) = self.rx.recv().await {
      match event {
        Event::Line(id, line) => {
          if let Some(proc) = self.processes.get(&id) {
            println!("{}", self.compose_line(proc, &line));
          }
        }

        Event::EOF(id) => {
          self.handle_exit(id);
          if self.processes.is_empty() {
            break;
          }
        }

        Event::CtrlC => {
          println!("\nCtrl+C received, stopping processes...");
          self.stop();
        }
      }
    }
  }

  fn log_spawn(&self, proc: &Process) {
    if let Some(pid) = proc.pid() {
      println!(
        "{}",
        self.compose_system_line(proc, &format!("Spawned, pid: {}", pid))
      );
    }
  }

  fn compose_system_line(&self, proc: &Process, line: &str) -> String {
    self.compose_line(proc, &format!("[{}] {}", proc.name, line))
  }

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
