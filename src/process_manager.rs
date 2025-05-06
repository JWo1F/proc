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
  Restart,
  Exit,
  #[value(skip)]
  Relax
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
}

static COLORS: [Color; 8] = [
  Color::Red, Color::Green, Color::Blue, Color::Cyan,
  Color::Magenta, Color::Yellow, Color::White, Color::BrightBlack,
];

impl ProcessManager {
  pub fn from_string(input: &str, mode: RunningMode) -> Result<Self, String> {
    let parsed = Self::parse_lines(input)?;
    let name_width = parsed.iter().map(|(name, _)| name.len()).max().unwrap_or(0);

    let processes = parsed
      .iter()
      .enumerate()
      .map(|(i, (name, cmd))| {
        let color = COLORS[i % COLORS.len()];
        Process::new(name, cmd, color)
      })
      .collect::<Vec<_>>();

    Ok(Self { processes, name_width, mode })
  }
  
  fn parse_lines(input: &str) -> Result<Vec<(&str, &str)>, String> {
    let mut result = Vec::new();
    let mut errors = Vec::new();
    
    for line in input.lines() {
      if line.starts_with('#') {
        continue;
      }

      let parsed = line.split_once(": ");

      match parsed {
        Some((name, cmd)) => {
          let name = name.trim();
          let cmd = cmd.trim();

          if name.is_empty() || cmd.is_empty() {
            errors.push(format!("Invalid line: {}", line));
            continue;
          }

          result.push((name, cmd));
        }

        None => {
          errors.push(format!("Invalid line: {}", line));
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

    let (res, index) = pipelines
      .race()
      .await;

    match res {
      ReadResult::Some(line) => {
        Some(self.compose_line(&self.processes[index], &line))
      }

      ReadResult::EOF => {
        self.restart(index);
        Box::pin(self.read_line()).await
      }

      ReadResult::Err(error) => {
        unimplemented!()
      }
    }
  }

  fn restart(&mut self, index: usize) {
    let proc = &self.processes[index];

    match self.mode {
      RunningMode::Restart => {
        println!("{}", self.compose_line(proc, "Restarting..."));
        self.processes[index].start();
      }

      RunningMode::Exit => {
        println!("{}", self.compose_line(proc, "Got Exit"));
        self.processes.remove(index);
        self.stop();
      }

      RunningMode::Relax => {
        println!("{}", self.compose_line(proc, "Got Relax"));
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
            Some(line) => {
              println!("{}", line);
            }

            None => {
              break;
            }
          }
        }

        _ = tokio::signal::ctrl_c() => {
          println!("Got Ctrl-C!");
          self.stop();
        }
      }
    }
  }

  fn compose_line(&self, proc: &Process, line: &str) -> String {
    format!(
      "{:width$} | {}",
      proc.name.color(proc.color),
      line,
      width = self.name_width
    )
  }
}
