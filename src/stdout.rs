use crate::core::LogEvent;
use colored::Colorize;
use crossterm::{cursor, execute, terminal};
use std::collections::HashSet;
use std::io::{Write, stdout};
use tokio::sync::{broadcast, mpsc, watch};

use crate::input::DisplayCommand;

const TIMESTAMP_FORMAT: &str = "%H:%M:%S";
const COMPACT_INDICATOR: &str = "▌";

pub struct StdoutConfig {
  pub timestamps: bool,
  pub compact: bool,
  pub no_system: bool,
  pub name_width: usize,
  /// Processes declared `muted`, hidden from the terminal from the first line.
  /// They still reach the web/SSE consumer, which reads the same broadcast.
  pub muted: Vec<String>,
  /// When Some, name_width updates dynamically (for the `add` command).
  pub name_width_rx: Option<watch::Receiver<usize>>,
  /// When Some, focus/mute/clear commands are accepted.
  pub display_rx: Option<mpsc::UnboundedReceiver<DisplayCommand>>,
  /// When Some and true, the modal owns the terminal — suppress printing so
  /// log lines don't corrupt its alternate screen.
  pub modal_active: Option<watch::Receiver<bool>>,
}

/// Which processes the terminal is currently showing.
///
/// Focus and mute are deliberately independent: focus is a temporary spotlight
/// that a single `focus off` undoes, while mute is a durable per-process
/// preference that should survive being focused and unfocused.
#[derive(Default)]
struct OutputFilter {
  focus: Option<String>,
  muted: HashSet<String>,
}

impl OutputFilter {
  /// Start out with the processes declared `muted` already hidden, so their
  /// very first line never reaches the terminal.
  fn new(muted: &[String]) -> Self {
    Self {
      focus: None,
      muted: muted.iter().cloned().collect(),
    }
  }

  fn shows(&self, process: &str) -> bool {
    if let Some(ref focused) = self.focus {
      return focused == process;
    }
    !self.muted.contains(process)
  }
}

pub async fn run(mut rx: broadcast::Receiver<LogEvent>, mut config: StdoutConfig) {
  let mut display_rx = config.display_rx.take();
  let mut filter = OutputFilter::new(&config.muted);

  loop {
    let received = match display_rx {
      Some(ref mut display_rx) => {
        tokio::select! {
          event = rx.recv() => event,
          command = display_rx.recv() => {
            if let Some(command) = command {
              apply_display(&mut filter, command, &config);
            }
            continue;
          }
        }
      }
      None => rx.recv().await,
    };

    match received {
      Ok(event) => {
        // Replies answer a command the user just typed, so they ignore both
        // --no-system and the focus/mute filter.
        if !event.reply {
          if config.no_system && event.system {
            continue;
          }
          if !filter.shows(&event.process) {
            continue;
          }
        }

        // Update name_width if it changed
        if let Some(ref mut nw_rx) = config.name_width_rx
          && nw_rx.has_changed().unwrap_or(false)
        {
          config.name_width = *nw_rx.borrow_and_update();
        }

        emit(&format_line(&event, &config), &config);
      }
      Err(broadcast::error::RecvError::Closed) => break,
      Err(broadcast::error::RecvError::Lagged(n)) => {
        emit(
          &format!("Warning: stdout dropped {} log events", n),
          &config,
        );
      }
    }
  }
}

/// Write one finished line, unless the modal currently owns the terminal.
///
/// Interactive mode holds the terminal in raw mode for the whole session, so
/// `\n` alone won't return the cursor to column 0 — every line needs an
/// explicit `\r`. Non-interactive mode never touches raw mode, so a plain
/// `println!` (relying on the terminal's own newline handling) is enough.
fn emit(line: &str, config: &StdoutConfig) {
  let Some(ref modal_active) = config.modal_active else {
    println!("{}", line);
    return;
  };
  if *modal_active.borrow() {
    return;
  }
  let mut out = stdout();
  let _ = write!(out, "{}\r\n", line);
  let _ = out.flush();
}

fn apply_display(filter: &mut OutputFilter, command: DisplayCommand, config: &StdoutConfig) {
  let confirmation = match command {
    DisplayCommand::Focus(Some(name)) => {
      let message = format!("Focused on {} — `focus off` to restore", name);
      filter.focus = Some(name);
      message
    }
    DisplayCommand::Focus(None) => {
      if filter.focus.take().is_none() {
        "Not focused on anything".to_string()
      } else {
        "Focus cleared".to_string()
      }
    }
    DisplayCommand::Mute(names) => {
      filter.muted.extend(names.iter().cloned());
      format!("Muted {}", names.join(", "))
    }
    DisplayCommand::Unmute(names) => {
      for name in &names {
        filter.muted.remove(name);
      }
      format!("Unmuted {}", names.join(", "))
    }
    DisplayCommand::UnmuteAll => {
      filter.muted.clear();
      "Unmuted everything".to_string()
    }
    DisplayCommand::Clear => {
      let mut out = stdout();
      let _ = execute!(
        out,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0),
      );
      let _ = out.flush();
      return;
    }
  };

  emit(&confirmation, config);
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

#[cfg(test)]
mod tests {
  use super::*;

  fn muting(names: &[&str]) -> HashSet<String> {
    names.iter().map(|n| n.to_string()).collect()
  }

  #[test]
  fn focus_hides_every_other_process() {
    let filter = OutputFilter {
      focus: Some("web".to_string()),
      ..Default::default()
    };

    assert!(filter.shows("web"));
    assert!(!filter.shows("worker"));
  }

  #[test]
  fn focus_outranks_mute_for_the_focused_process() {
    let filter = OutputFilter {
      focus: Some("web".to_string()),
      muted: muting(&["web"]),
    };

    assert!(filter.shows("web"));
  }

  #[test]
  fn mute_survives_a_focus_round_trip() {
    let mut filter = OutputFilter {
      focus: Some("web".to_string()),
      muted: muting(&["css"]),
    };
    filter.focus = None;

    assert!(!filter.shows("css"));
    assert!(filter.shows("web"));
  }

  #[test]
  fn a_muted_declaration_hides_the_process_from_the_first_line() {
    let filter = OutputFilter::new(&["logs".to_string()]);

    assert!(!filter.shows("logs"));
    assert!(filter.shows("web"));

    // And it is an ordinary mute, so unmuting from the menu lifts it.
    let mut filter = filter;
    filter.muted.remove("logs");
    assert!(filter.shows("logs"));
  }

  #[test]
  fn nothing_is_hidden_by_default() {
    let filter = OutputFilter::default();
    assert!(filter.shows("web"));
  }
}
