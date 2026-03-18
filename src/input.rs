use crate::core::LogEvent;
use colored::Color;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use crossterm::{cursor, execute, terminal as term};
use std::io::{Write, stdout};
use tokio::sync::{broadcast, mpsc, watch};

/// A parsed interactive command.
#[derive(Debug, PartialEq)]
pub enum Command {
    Kill(String),
    Restart(String),
    Run(String, String),
    Up(String),
    Down(String),
    List,
    Help,
}

/// Events sent from the input module to the manager.
pub enum InputEvent {
    Command(Command),
    CtrlC,
}

/// Parse a user input line into a Command.
pub fn parse_command(input: &str) -> Result<Command, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err(String::new());
    }

    let (cmd, rest) = match input.split_once(' ') {
        Some((c, r)) => (c, r.trim()),
        None => (input, ""),
    };

    match cmd {
        "kill" => {
            if rest.is_empty() {
                return Err("Usage: kill <name>".to_string());
            }
            Ok(Command::Kill(rest.to_string()))
        }
        "restart" => {
            if rest.is_empty() {
                return Err("Usage: restart <name>".to_string());
            }
            Ok(Command::Restart(rest.to_string()))
        }
        "run" => {
            let Some((name, cmd)) = rest.split_once(':') else {
                return Err("Usage: run <name>: <command>".to_string());
            };
            let name = name.trim();
            let cmd = cmd.trim();
            if name.is_empty() || cmd.is_empty() {
                return Err("Usage: run <name>: <command>".to_string());
            }
            Ok(Command::Run(name.to_string(), cmd.to_string()))
        }
        "up" => {
            if rest.is_empty() {
                return Err("Usage: up <name>".to_string());
            }
            Ok(Command::Up(rest.to_string()))
        }
        "down" => {
            if rest.is_empty() {
                return Err("Usage: down <name>".to_string());
            }
            Ok(Command::Down(rest.to_string()))
        }
        "list" | "ps" => Ok(Command::List),
        "help" => Ok(Command::Help),
        _ => Err(format!("Unknown command: {}", cmd)),
    }
}

pub const PROMPT: &str = "procfile> ";

/// RAII guard that enables raw mode on creation and restores on drop.
pub struct RawModeGuard;

impl RawModeGuard {
    pub fn new() -> std::io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

/// Install a panic hook that disables raw mode before printing the panic.
pub fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = terminal::disable_raw_mode();
        default_hook(info);
    }));
}

/// Draw the prompt with the current buffer contents.
fn draw_prompt(buffer: &str) {
    let mut out = stdout();
    let _ = execute!(
        out,
        cursor::MoveToColumn(0),
        term::Clear(term::ClearType::CurrentLine),
    );
    let _ = write!(out, "{}{}", PROMPT, buffer);
    let _ = out.flush();
}

/// Draw an error message on the prompt line.
fn draw_error(message: &str) {
    let mut out = stdout();
    let _ = execute!(
        out,
        cursor::MoveToColumn(0),
        term::Clear(term::ClearType::CurrentLine),
    );
    let _ = write!(out, "{}", message);
    let _ = out.flush();
}

/// Emit help text through the broadcast channel as system messages.
fn emit_help(log_tx: &broadcast::Sender<LogEvent>) {
    let help_lines = [
        "Available commands:",
        "  kill <name>           Send SIGINT to a process",
        "  restart <name>        Kill and re-spawn a process",
        "  run <name>: <command> Add and start a new process",
        "  up <name>             Start a stopped process",
        "  down <name>           Stop and remove a process",
        "  list                  Show running processes",
        "  help                  Show this help",
    ];

    for line in help_lines {
        let _ = log_tx.send(LogEvent {
            process: "system".to_string(),
            color: Color::White,
            line: line.to_string(),
            system: true,
        });
    }
}

/// Run the input event loop. Reads keypresses, parses commands, sends events.
pub async fn run(
    input_tx: mpsc::UnboundedSender<InputEvent>,
    buffer_tx: watch::Sender<String>,
    log_tx: broadcast::Sender<LogEvent>,
) {
    let mut buffer = String::new();
    let mut showing_error = false;
    let mut history: Vec<String> = Vec::new();
    let mut history_pos: Option<usize> = None; // None = typing new input
    let mut saved_input = String::new(); // saves current input when browsing history

    draw_prompt(&buffer);

    loop {
        // Read next terminal event in a blocking thread
        let event = match tokio::task::spawn_blocking(crossterm::event::read).await {
            Ok(Ok(event)) => event,
            _ => break,
        };

        let Event::Key(KeyEvent { code, modifiers, kind, .. }) = event else {
            continue;
        };

        // crossterm may send both Press and Release events; only handle Press
        if kind != KeyEventKind::Press {
            continue;
        }

        // Clear error display on any keypress
        if showing_error {
            showing_error = false;
            buffer.clear();
            let _ = buffer_tx.send(buffer.clone());
            if code == KeyCode::Enter {
                draw_prompt(&buffer);
                continue;
            }
        }

        match (code, modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                let _ = input_tx.send(InputEvent::CtrlC);
            }
            (KeyCode::Char(c), _) => {
                buffer.push(c);
                let _ = buffer_tx.send(buffer.clone());
                draw_prompt(&buffer);
            }
            (KeyCode::Backspace, _) => {
                buffer.pop();
                let _ = buffer_tx.send(buffer.clone());
                draw_prompt(&buffer);
            }
            (KeyCode::Up, _) => {
                if history.is_empty() {
                    continue;
                }
                match history_pos {
                    None => {
                        saved_input = buffer.clone();
                        history_pos = Some(history.len() - 1);
                    }
                    Some(pos) if pos > 0 => {
                        history_pos = Some(pos - 1);
                    }
                    _ => continue,
                }
                buffer = history[history_pos.unwrap()].clone();
                let _ = buffer_tx.send(buffer.clone());
                draw_prompt(&buffer);
            }
            (KeyCode::Down, _) => {
                let Some(pos) = history_pos else {
                    continue;
                };
                if pos + 1 < history.len() {
                    history_pos = Some(pos + 1);
                    buffer = history[pos + 1].clone();
                } else {
                    history_pos = None;
                    buffer = saved_input.clone();
                }
                let _ = buffer_tx.send(buffer.clone());
                draw_prompt(&buffer);
            }
            (KeyCode::Enter, _) => {
                let input = buffer.clone();
                buffer.clear();
                history_pos = None;
                saved_input.clear();
                let _ = buffer_tx.send(buffer.clone());

                if !input.trim().is_empty() {
                    // Don't add duplicates of the last entry
                    if history.last().map(|h| h.as_str()) != Some(input.trim()) {
                        history.push(input.trim().to_string());
                    }
                }

                match parse_command(&input) {
                    Ok(Command::Help) => {
                        emit_help(&log_tx);
                        draw_prompt(&buffer);
                    }
                    Ok(cmd) => {
                        let _ = input_tx.send(InputEvent::Command(cmd));
                        draw_prompt(&buffer);
                    }
                    Err(msg) => {
                        if msg.is_empty() {
                            draw_prompt(&buffer);
                        } else {
                            draw_error(&msg);
                            showing_error = true;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kill() {
        assert_eq!(parse_command("kill web"), Ok(Command::Kill("web".to_string())));
    }

    #[test]
    fn parse_restart() {
        assert_eq!(parse_command("restart web"), Ok(Command::Restart("web".to_string())));
    }

    #[test]
    fn parse_run_with_colon() {
        assert_eq!(
            parse_command("run worker: bundle exec sidekiq"),
            Ok(Command::Run("worker".to_string(), "bundle exec sidekiq".to_string()))
        );
    }

    #[test]
    fn parse_run_missing_colon() {
        assert!(parse_command("run worker").is_err());
    }

    #[test]
    fn parse_up() {
        assert_eq!(parse_command("up web"), Ok(Command::Up("web".to_string())));
    }

    #[test]
    fn parse_down() {
        assert_eq!(parse_command("down web"), Ok(Command::Down("web".to_string())));
    }

    #[test]
    fn parse_list() {
        assert_eq!(parse_command("list"), Ok(Command::List));
    }

    #[test]
    fn parse_ps_alias() {
        assert_eq!(parse_command("ps"), Ok(Command::List));
    }

    #[test]
    fn parse_help() {
        assert_eq!(parse_command("help"), Ok(Command::Help));
    }

    #[test]
    fn parse_unknown_command() {
        assert!(parse_command("foo").is_err());
    }

    #[test]
    fn parse_empty_input() {
        assert!(parse_command("").is_err());
    }

    #[test]
    fn parse_whitespace_only() {
        assert!(parse_command("   ").is_err());
    }

    #[test]
    fn parse_kill_missing_name() {
        assert!(parse_command("kill").is_err());
    }
}
