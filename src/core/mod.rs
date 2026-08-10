pub mod ansi;
pub mod color;
pub mod manager;
pub mod process;
pub mod procfile;

use colored::Color;

/// A log event emitted by the process manager.
/// Consumers (stdout, web) receive these via broadcast channel.
#[derive(Clone, Debug)]
pub struct LogEvent {
  /// The process name that produced this line.
  pub process: String,
  /// Terminal color assigned to the process.
  pub color: Color,
  /// The log line content (may contain ANSI color codes).
  pub line: String,
  /// Whether this is a system message (spawn, exit, etc).
  pub system: bool,
  /// Whether this is a direct reply to an interactive command (`ps`, `help`,
  /// `info`). Replies are system messages that `--no-system` must not hide —
  /// suppressing output the user explicitly asked for reads as a broken prompt.
  pub reply: bool,
}
