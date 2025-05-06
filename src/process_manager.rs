use crate::process::{Process, ReadResult};
use colored::{Color, Colorize};
use futures_concurrency::future::Race;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::collections::HashMap;

#[derive(PartialOrd, PartialEq)]
pub enum RunningMode {
  Restart,
  Exit,
  Relax
}

pub struct ProcessManager {
  mode: RunningMode,
  name_width: usize,
  processes: HashMap<String, Process>,
}

static COLORS: [Color; 8] = [
  Color::Red, Color::Green, Color::Blue, Color::Cyan,
  Color::Magenta, Color::Yellow, Color::White, Color::BrightBlack,
];

impl ProcessManager {
  pub fn from_string(input: &str) -> Result<Self, String> {
    let mut processes = HashMap::new();
    let mut errors = Vec::new();

    for line in input.lines() {
      let parsed = line.split_once(": ");
      
      if parsed.is_none() {
        errors.push(format!("Invalid line: {}", line));
        continue;
      }
      
      let (name, cmd) = parsed.unwrap();
      let name = name.trim();
      let cmd = cmd.trim();
      
      if name.is_empty() || cmd.is_empty() {
        errors.push(format!("Invalid line: {}", line));
        continue;
      }
      
      if errors.is_empty() {
        let color = COLORS[processes.len() % COLORS.len()];
        let process = Process::new(name, cmd, color);

        processes.insert(name.to_string(), process);
      }
    }

    if errors.is_empty() {
      let name_width = processes.values().map(|p| p.name.len()).max().unwrap_or(0);
      let mode = RunningMode::Exit;

      Ok(Self { processes, name_width, mode })
    } else {
      Err(errors.join("\n"))
    }
  }
  
  pub fn spawn(&mut self) {
    for process in self.processes.values_mut() {
      process.spawn();
    }
  }

  pub async fn read_line(&mut self) -> (ReadResult, &Process) {
    let (res, proc_name) = self.processes
      .values_mut()
      .map(|p| async { (p.read_line().await, p.name.clone()) })
      .collect::<Vec<_>>()
      .race()
      .await;

    let res = match res {
      ReadResult::Some(line) => {
        let proc = &self.processes[&proc_name];
        
        ReadResult::Some(
          format!(
            "{:width$} | {}",
            proc.name.color(proc.color),
            line,
            width = self.name_width
          )
        )
      }
      
      ReadResult::EOF => {
        self.restart(&proc_name);
        return Box::pin(self.read_line()).await;
      }

      other => other
    };

    (res, self.processes.get(&proc_name).unwrap())
  }
  
  fn restart(&mut self, proc_name: &str) {
    match self.mode {
      RunningMode::Restart => {
        println!("Got Restart");
      }
      
      RunningMode::Exit => {
        println!("Got Exit");
      }
      
      RunningMode::Relax => {
        println!("Got Relax");
        self.processes.remove(proc_name);
      }
    }
  }

  pub async fn stop(&mut self) {
    self.mode = RunningMode::Relax;
    
    for process in self.processes.values() {
      let id = process.pair.as_ref().unwrap().1.id().unwrap();
      let pid = Pid::from_raw(id as i32);
      println!("Stopping pid {}", pid);
      kill(pid, Signal::SIGINT).unwrap();
    }

    // for process in self.processes.values_mut() {
    //   process.pair.as_mut().unwrap().1.wait().await.unwrap();
    //   println!("Process {} exited", process.name);
    // }
  }
}
