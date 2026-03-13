//! Pipe mode: reads lines from stdin and emits them as LogEvents.
//!
//! Used when procfile receives piped input, e.g.:
//!   my_command | procfile
//!   ssh host "cmd" | procfile -w

use crate::core::LogEvent;
use colored::Color;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::broadcast;

const STDIN_NAME: &str = "stdin";
const STDIN_COLOR: Color = Color::Cyan;

fn stdin_event(line: String, system: bool) -> LogEvent {
  LogEvent {
    process: STDIN_NAME.to_string(),
    color: STDIN_COLOR,
    line,
    system,
  }
}

/// Read lines from stdin and emit them as LogEvents.
/// Returns 0 on clean EOF, 1 on error.
pub async fn run(log_tx: broadcast::Sender<LogEvent>) -> u8 {
  let stdin = tokio::io::stdin();
  let mut reader = BufReader::new(stdin).lines();

  let _ = log_tx.send(stdin_event(
    format!("[{}] Reading from pipe...", STDIN_NAME),
    true,
  ));

  let mut failed = false;

  loop {
    match reader.next_line().await {
      Ok(Some(line)) => {
        let _ = log_tx.send(stdin_event(line, false));
      }
      Ok(None) => break,
      Err(err) => {
        let _ = log_tx.send(stdin_event(
          format!("[{}] Read error: {}", STDIN_NAME, err),
          true,
        ));
        failed = true;
        break;
      }
    }
  }

  let _ = log_tx.send(stdin_event(
    format!("[{}] End of input", STDIN_NAME),
    true,
  ));

  if failed { 1 } else { 0 }
}
