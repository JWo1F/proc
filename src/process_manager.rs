use std::fmt::Display;
use clap::ValueEnum;
use crate::process::{Process, ReadResult};
use crate::signal::ChildSignal;
use colored::{Color, Colorize};
use futures_concurrency::future::Race;
use nix::sys::signal::Signal;
use tokio::select;

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

pub struct ProcessManager {
  mode: RunningMode,
  name_width: usize,
  processes: Vec<Process>,
  timestamps: bool,
}

static COLORS: [Color; 8] = [
  Color::Red, Color::Green, Color::Blue, Color::Cyan,
  Color::Magenta, Color::Yellow, Color::White, Color::BrightBlack,
];

impl ProcessManager {
  pub fn from_string(input: &str, mode: RunningMode, exclude: &[String], include: &[String], timestamps: bool) -> Result<Self, String> {
    let parsed = Self::parse_lines(input, exclude, include)?;
    let name_width = parsed.iter().map(|(name, _)| name.len()).max().unwrap_or(0);

    let processes = parsed
      .iter()
      .enumerate()
      .map(|(i, (name, cmd))| {
        let color = COLORS[i % COLORS.len()];
        Process::new(name, cmd, color)
      })
      .collect::<Vec<_>>();

    let manager = Self { processes, name_width, mode, timestamps };

    for proc in &manager.processes {
      manager.log_spawn(proc);
    }

    Ok(manager)
  }

  fn parse_lines<'a>(input: &'a str, exclude: &[String], include: &[String]) -> Result<Vec<(&'a str, &'a str)>, String> {
    let mut result = Vec::new();
    let mut errors = Vec::new();

    for (n, line) in input.lines().enumerate() {
      if line.starts_with('#') || line.trim().is_empty() {
        continue;
      }

      let n = n + 1;

      match line.split_once(":") {
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

          let (name, disabled) = match name.strip_prefix('$') {
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
          errors.push(format!("Line {} should contain name and command:\n> {}", n, line));
        }
      }
    }

    if errors.is_empty() {
      Ok(result)
    } else {
      Err(errors.join("\n"))
    }
  }

  pub async fn read_line(&mut self) -> Option<String> {
    let pipelines = self.processes
      .iter_mut()
      .enumerate()
      .map(|(i, p)| async move { (p.read_line().await, i) })
      .collect::<Vec<_>>();

    if pipelines.is_empty() {
      return None;
    }

    let (res, index) = pipelines.race().await;

    match res {
      ReadResult::Some(line) => {
        Some(self.compose_line(&self.processes[index], &line))
      }

      ReadResult::EOF => {
        self.handle_exit(index);
        Box::pin(self.read_line()).await
      }

      ReadResult::Err(error) => {
        eprintln!("Error reading process output: {}", error);
        None
      }
    }
  }

  fn handle_exit(&mut self, index: usize) {
    let proc = &self.processes[index];

    match self.mode {
      RunningMode::Restart => {
        println!("{}", self.compose_line(proc, "Restarting..."));
        self.processes[index].start();
        self.log_spawn(&self.processes[index]);
      }

      RunningMode::Exit => {
        println!("{}", self.compose_line(proc, "Exited"));
        self.processes.remove(index);
        self.stop();
      }

      RunningMode::Relax => {
        println!("{}", self.compose_line(proc, "Stopped"));
        self.processes.remove(index);
      }
    }
  }

  pub fn stop(&mut self) {
    self.mode = RunningMode::Relax;

    for process in self.processes.iter() {
      if let Some(child) = &process.child {
        child.signal(Signal::SIGINT);
        println!("{}", self.compose_line(process, "Stopping..."));
      }
    }
  }

  pub async fn start(&mut self) {
    loop {
      select! {
        line = self.read_line() => {
          match line {
            Some(line) => println!("{}", line),
            None => break,
          }
        }

        _ = tokio::signal::ctrl_c() => {
          println!("Ctrl+C received, stopping processes...");
          self.stop();
        }
      }
    }
  }

  fn log_spawn(&self, proc: &Process) {
    if let Some(pid) = proc.pid() {
      println!("{}", self.compose_line(proc, &format!("Spawned, pid: {}", pid)));
    }
  }

  fn compose_line(&self, proc: &Process, line: &str) -> String {
    let name = proc.name.color(proc.color);
    let width = self.name_width;

    if self.timestamps {
      let now = chrono::Local::now();
      let timestamp = now.format("%H:%M:%S").to_string().color(proc.color);
      format!("{} | {:width$} | {}", timestamp, name, line)
    } else {
      format!("{:width$} | {}", name, line)
    }
  }
}
