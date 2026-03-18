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
    Add(String, String),
    Remove(String),
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
        "add" => {
            let Some((name, cmd)) = rest.split_once(':') else {
                return Err("Usage: add <name>: <command>".to_string());
            };
            let name = name.trim();
            let cmd = cmd.trim();
            if name.is_empty() || cmd.is_empty() {
                return Err("Usage: add <name>: <command>".to_string());
            }
            Ok(Command::Add(name.to_string(), cmd.to_string()))
        }
        "remove" => {
            if rest.is_empty() {
                return Err("Usage: remove <name>".to_string());
            }
            Ok(Command::Remove(rest.to_string()))
        }
        "list" | "ps" => Ok(Command::List),
        "help" => Ok(Command::Help),
        _ => Err(format!("Unknown command: {}", cmd)),
    }
}

const PROMPT: &str = "> ";

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
        "  add <name>: <command> Add and start a new process",
        "  remove <name>         Kill and remove a process",
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
            (KeyCode::Enter, _) => {
                let input = buffer.clone();
                buffer.clear();
                let _ = buffer_tx.send(buffer.clone());

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
    fn parse_add_with_colon() {
        assert_eq!(
            parse_command("add worker: bundle exec sidekiq"),
            Ok(Command::Add("worker".to_string(), "bundle exec sidekiq".to_string()))
        );
    }

    #[test]
    fn parse_add_missing_colon() {
        assert!(parse_command("add worker").is_err());
    }

    #[test]
    fn parse_remove() {
        assert_eq!(parse_command("remove web"), Ok(Command::Remove("web".to_string())));
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
