use crate::core::LogEvent;
use colored::Colorize;
use crossterm::{cursor, execute, terminal};
use std::io::{Write, stdout};
use tokio::sync::{broadcast, watch};

use crate::input::PROMPT;

const TIMESTAMP_FORMAT: &str = "%H:%M:%S";
const COMPACT_INDICATOR: &str = "▌";

pub struct StdoutConfig {
  pub timestamps: bool,
  pub compact: bool,
  pub no_system: bool,
  pub name_width: usize,
  /// When Some, interactive mode is active — read buffer from the watch channel.
  pub interactive: Option<watch::Receiver<String>>,
  /// When Some, name_width updates dynamically (for `add` command).
  pub name_width_rx: Option<watch::Receiver<usize>>,
}

pub async fn run(mut rx: broadcast::Receiver<LogEvent>, mut config: StdoutConfig) {
  loop {
    match rx.recv().await {
      Ok(event) => {
        if config.no_system && event.system {
          continue;
        }

        // Update name_width if it changed
        if let Some(ref mut nw_rx) = config.name_width_rx {
          if nw_rx.has_changed().unwrap_or(false) {
            config.name_width = *nw_rx.borrow_and_update();
          }
        }

        let line = format_line(&event, &config);

        if let Some(ref buffer_rx) = config.interactive {
          // Interactive mode: print above prompt, then redraw prompt
          let buffer = buffer_rx.borrow().clone();
          let mut out = stdout();
          let _ = execute!(
            out,
            cursor::MoveToColumn(0),
            terminal::Clear(terminal::ClearType::CurrentLine),
          );
          let _ = write!(out, "{}\r\n", line);
          let _ = write!(out, "{}{}", PROMPT, buffer);
          let _ = out.flush();
        } else {
          println!("{}", line);
        }
      }
      Err(broadcast::error::RecvError::Closed) => break,
      Err(broadcast::error::RecvError::Lagged(n)) => {
        if config.interactive.is_some() {
          let buffer = config
            .interactive
            .as_ref()
            .map(|rx| rx.borrow().clone())
            .unwrap_or_default();
          let mut out = stdout();
          let _ = execute!(
            out,
            cursor::MoveToColumn(0),
            terminal::Clear(terminal::ClearType::CurrentLine),
          );
          let _ = write!(out, "Warning: stdout dropped {} log events\r\n", n);
          let _ = write!(out, "{}{}", PROMPT, buffer);
          let _ = out.flush();
        } else {
          eprintln!("Warning: stdout dropped {} log events", n);
        }
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
