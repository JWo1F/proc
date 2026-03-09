use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

#[cfg(feature = "web")]
use tokio::sync::broadcast;

/// A single log entry with a unique sequential index.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct LogEntry {
  /// Sequential index (0-based), used as the log ID.
  pub index: usize,
  /// The process name that produced this line.
  pub process: String,
  /// CSS color string for the process (e.g. "rgb(r,g,b)").
  pub color: String,
  /// The raw log line content (may contain ANSI codes).
  pub line: String,
  /// Timestamp when the line was recorded.
  pub timestamp: String,
  /// Whether this is a system message (spawn, exit, etc).
  pub system: bool,
}

/// Thread-safe log store with pub/sub for new entries.
pub struct LogStore {
  entries: Arc<RwLock<Vec<LogEntry>>>,
  counter: AtomicUsize,
  #[cfg(feature = "web")]
  sender: broadcast::Sender<LogEntry>,
}

impl LogStore {
  #[cfg(feature = "web")]
  pub fn new() -> Self {
    let (sender, _) = broadcast::channel(1024);
    Self {
      entries: Arc::new(RwLock::new(Vec::new())),
      counter: AtomicUsize::new(0),
      sender,
    }
  }

  #[cfg(not(feature = "web"))]
  pub fn new() -> Self {
    Self {
      entries: Arc::new(RwLock::new(Vec::new())),
      counter: AtomicUsize::new(0),
    }
  }

  /// Push a new log entry and notify subscribers.
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

    #[cfg(feature = "web")]
    let _ = self.sender.send(entry);
  }

  /// Get all stored entries.
  #[cfg(feature = "web")]
  pub fn get_all(&self) -> Vec<LogEntry> {
    self.entries.read().map(|e| e.clone()).unwrap_or_default()
  }

  /// Subscribe to new log entries via broadcast channel.
  #[cfg(feature = "web")]
  pub fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
    self.sender.subscribe()
  }
}
