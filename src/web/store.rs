use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

/// A single log entry stored for web clients.
#[derive(Clone, Debug)]
pub(super) struct LogEntry {
  pub index: usize,
  pub process: String,
  pub color: String,
  pub line: String,
  pub timestamp: String,
  pub system: bool,
}

/// Thread-safe log store with pub/sub for SSE clients.
pub(super) struct LogStore {
  entries: Arc<RwLock<Vec<LogEntry>>>,
  counter: AtomicUsize,
  sender: broadcast::Sender<LogEntry>,
}

impl LogStore {
  pub fn new() -> Self {
    let (sender, _) = broadcast::channel(1024);
    Self {
      entries: Arc::new(RwLock::new(Vec::new())),
      counter: AtomicUsize::new(0),
      sender,
    }
  }

  pub fn push(&self, process: &str, color: &str, line: &str, timestamp: &str, system: bool) {
    let index = self.counter.fetch_add(1, Ordering::Relaxed);
    let entry = LogEntry {
      index,
      process: process.to_string(),
      color: color.to_string(),
      line: line.to_string(),
      timestamp: timestamp.to_string(),
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

  pub fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
    self.sender.subscribe()
  }
}
