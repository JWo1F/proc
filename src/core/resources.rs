//! Background resource sampling for the interactive modal's process Info
//! view. Runs on its own OS thread (not a tokio task) since `sysinfo`'s
//! refresh is a blocking syscall and this app's runtime is single-threaded —
//! blocking it here would stall every PTY reader and the log broadcast.

use super::manager::Snapshot;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::sync::watch;

pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const HISTORY_LEN: usize = 60;

/// A single point-in-time reading for one process's whole group (the shell
/// plus everything it has spawned).
#[derive(Clone, Debug, Default)]
pub struct ResourceSample {
  pub mem_bytes: u64,
  pub cpu_percent: f32,
  pub process_count: usize,
  pub thread_count: usize,
}

/// The latest sample plus a rolling window of memory/CPU readings, for the
/// Info view's sparklines.
#[derive(Clone, Debug, Default)]
pub struct ResourceHistory {
  pub current: ResourceSample,
  pub mem_history: VecDeque<u64>,
  pub cpu_history: VecDeque<u64>,
}

impl ResourceHistory {
  fn push(&mut self, sample: ResourceSample) {
    push_capped(&mut self.mem_history, sample.mem_bytes / (1024 * 1024));
    push_capped(&mut self.cpu_history, sample.cpu_percent.round() as u64);
    self.current = sample;
  }
}

fn push_capped(history: &mut VecDeque<u64>, value: u64) {
  if history.len() == HISTORY_LEN {
    history.pop_front();
  }
  history.push_back(value);
}

pub type ResourceMap = HashMap<String, ResourceHistory>;

/// Sample every tracked process's group once per `SAMPLE_INTERVAL`, for as
/// long as `snapshot_rx` keeps producing values (i.e., for the life of the
/// interactive session).
pub fn run_sampler(mut snapshot_rx: watch::Receiver<Snapshot>, resources_tx: watch::Sender<ResourceMap>) {
  let mut system = System::new();
  let mut history: ResourceMap = HashMap::new();
  let refresh_kind = ProcessRefreshKind::nothing()
    .with_memory()
    .with_cpu()
    .with_tasks();

  loop {
    let snapshot = snapshot_rx.borrow_and_update().clone();

    system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind);
    let children = children_index(&system);

    history.retain(|name, _| snapshot.processes.iter().any(|p| &p.name == name));

    for proc in &snapshot.processes {
      let Some(pid) = proc.pid else { continue };
      let sample = sample_group(&system, &children, Pid::from_u32(pid));
      history.entry(proc.name.clone()).or_default().push(sample);
    }

    if resources_tx.send(history.clone()).is_err() {
      break;
    }

    std::thread::sleep(SAMPLE_INTERVAL);
  }
}

/// Map each process to its direct children, from the whole-system snapshot.
fn children_index(system: &System) -> HashMap<Pid, Vec<Pid>> {
  let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
  for (pid, process) in system.processes() {
    if let Some(parent) = process.parent() {
      children.entry(parent).or_default().push(*pid);
    }
  }
  children
}

/// Sum memory/CPU/thread counts across `root` and every descendant found by
/// walking the parent/child tree — "the shell plus whatever it spawns".
fn sample_group(system: &System, children: &HashMap<Pid, Vec<Pid>>, root: Pid) -> ResourceSample {
  let mut seen = HashSet::new();
  let mut queue = VecDeque::from([root]);
  let mut sample = ResourceSample::default();

  while let Some(pid) = queue.pop_front() {
    if !seen.insert(pid) {
      continue;
    }
    if let Some(process) = system.process(pid) {
      sample.mem_bytes += process.memory();
      sample.cpu_percent += process.cpu_usage();
      // `tasks()` (thread ids) is Linux-only in sysinfo; stays 0 elsewhere,
      // which the Info view renders as "unavailable" rather than a real count.
      sample.thread_count += process.tasks().map_or(0, |tasks| tasks.len());
      sample.process_count += 1;
    }
    if let Some(kids) = children.get(&pid) {
      queue.extend(kids.iter().copied());
    }
  }

  sample
}
