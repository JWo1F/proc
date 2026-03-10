use crate::core::LogEvent;
use colored::Colorize;
use tokio::sync::broadcast;

const TIMESTAMP_FORMAT: &str = "%H:%M:%S";
const COMPACT_INDICATOR: &str = "▌";

pub struct StdoutConfig {
  pub timestamps: bool,
  pub compact: bool,
  pub name_width: usize,
}

/// Run the stdout log consumer. Reads log events from the broadcast channel
/// and prints them to stdout with colored prefixes.
pub async fn run(mut rx: broadcast::Receiver<LogEvent>, config: StdoutConfig) {
  loop {
    match rx.recv().await {
      Ok(event) => {
        println!("{}", format_line(&event, &config));
      }
      Err(broadcast::error::RecvError::Closed) => break,
      Err(broadcast::error::RecvError::Lagged(n)) => {
        eprintln!("Warning: stdout dropped {} log events", n);
      }
    }
  }
}

fn format_line(event: &LogEvent, config: &StdoutConfig) -> String {
  let mut parts = Vec::new();

  if config.timestamps {
    let now = chrono::Local::now();
    parts.push(
      now
        .format(TIMESTAMP_FORMAT)
        .to_string()
        .color(event.color)
        .to_string(),
    );
  }

  if config.compact {
    parts.push(COMPACT_INDICATOR.color(event.color).to_string());
  } else {
    let width = config.name_width;
    parts.push(
      format!("{:width$} |", event.process)
        .color(event.color)
        .to_string(),
    );
  }

  format!("{} {}", parts.join(" "), event.line)
}
