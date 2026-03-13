use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

/// A registered process with its assigned ID and CSS color.
#[derive(Clone, Debug)]
pub(super) struct ProcessInfo {
  pub id: usize,
  pub name: String,
  pub color: String,
}

/// A single log entry stored for web clients.
#[derive(Clone, Debug)]
pub(super) struct LogEntry {
  pub index: usize,
  pub process_id: usize,
  pub line: String,
  pub timestamp: f64,
  pub system: bool,
}

/// Thread-safe log store with pub/sub for SSE clients.
pub(super) struct LogStore {
  entries: Arc<RwLock<Vec<LogEntry>>>,
  processes: Arc<RwLock<Vec<ProcessInfo>>>,
  process_map: Arc<RwLock<HashMap<String, usize>>>,
  counter: AtomicUsize,
  sender: broadcast::Sender<LogEntry>,
}

impl LogStore {
  pub fn new() -> Self {
    let (sender, _) = broadcast::channel(1024);
    Self {
      entries: Arc::new(RwLock::new(Vec::new())),
      processes: Arc::new(RwLock::new(Vec::new())),
      process_map: Arc::new(RwLock::new(HashMap::new())),
      counter: AtomicUsize::new(0),
      sender,
    }
  }

  /// Register a process and return its ID. If already registered, returns existing ID.
  pub fn register_process(&self, name: &str, css_color: &str) -> usize {
    if let Ok(map) = self.process_map.read() {
      if let Some(&id) = map.get(name) {
        return id;
      }
    }
    let mut map = self.process_map.write().unwrap();
    // Double-check after acquiring write lock
    if let Some(&id) = map.get(name) {
      return id;
    }
    let mut procs = self.processes.write().unwrap();
    let id = procs.len();
    procs.push(ProcessInfo {
      id,
      name: name.to_string(),
      color: css_color.to_string(),
    });
    map.insert(name.to_string(), id);
    id
  }

  pub fn push(&self, process_id: usize, line: &str, timestamp: f64, system: bool) {
    let index = self.counter.fetch_add(1, Ordering::Relaxed);
    let entry = LogEntry {
      index,
      process_id,
      line: line.to_string(),
      timestamp,
      system,
    };

    if let Ok(mut entries) = self.entries.write() {
      entries.push(entry.clone());
    }

    let _ = self.sender.send(entry);
  }

  pub fn get_all(&self) -> Vec<LogEntry> {
    self.entries.read().map(|e| e.clone()).unwrap_or_default()
  }

  pub fn get_processes(&self) -> Vec<ProcessInfo> {
    self.processes.read().map(|p| p.clone()).unwrap_or_default()
  }

  pub fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
    self.sender.subscribe()
  }
}
