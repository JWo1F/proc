use crate::core::LogEvent;
use crate::core::manager::OnExit;
use colored::Color;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use crossterm::{cursor, execute, terminal as term};
use std::io::{Write, stdout};
use tokio::sync::{broadcast, mpsc, watch};

/// The set of processes a lifecycle command applies to.
#[derive(Debug, PartialEq, Clone)]
pub enum Target {
  All,
  Names(Vec<String>),
}

/// A command that mutates or inspects process state. Handled by the manager.
#[derive(Debug, PartialEq)]
pub enum Command {
  Start(Target),
  Stop(Target),
  Restart(Target),
  Kill(Target),
  Add(String, String),
  Remove(Target),
  Ps,
  Info(String),
  /// `None` reports the current policy instead of changing it. Worth asking
  /// about, since the default depends on whether the prompt is attached.
  Mode(Option<OnExit>),
  Quit,
}

/// A command that only affects how output is rendered. Handled by the stdout
/// consumer, never by the manager — filtering the broadcast itself would
/// starve the web/SSE consumer of lines it never asked to hide.
#[derive(Debug, PartialEq, Clone)]
pub enum DisplayCommand {
  /// `Some(name)` shows only that process; `None` restores everything.
  Focus(Option<String>),
  Mute(Vec<String>),
  Unmute(Vec<String>),
  UnmuteAll,
  Clear,
}

/// Events sent from the input module to the manager.
pub enum InputEvent {
  Command(Command),
  CtrlC,
}

/// A parsed line, routed to whichever subsystem owns it.
#[derive(Debug, PartialEq)]
pub enum Parsed {
  Manager(Command),
  Display(DisplayCommand),
  /// `help`, optionally scoped to a single command.
  Help(Option<String>),
}

/// Every command name, in help/completion order.
pub const COMMANDS: &[&str] = &[
  "start", "stop", "restart", "kill", "add", "remove", "ps", "info", "focus", "mute", "unmute",
  "clear", "mode", "help", "quit",
];

/// Commands from the pre-2.8 vocabulary, mapped to their replacement so a
/// stale reflex gets a redirect instead of a bare "unknown command".
const RENAMED: &[(&str, &str)] = &[
  ("up", "start"),
  ("down", "stop"),
  ("run", "add"),
  ("list", "ps"),
  ("ls", "ps"),
];

/// One-line summaries, used by both the grouped help and `help <command>`.
const HELP: &[(&str, &str, &str)] = &[
  ("start", "<targets>", "Spawn stopped processes"),
  (
    "stop",
    "<targets>",
    "Graceful stop (SIGINT); stays in the table",
  ),
  (
    "restart",
    "<targets>",
    "Stop then start, resetting restart backoff",
  ),
  ("kill", "<targets>", "Immediate SIGKILL, no grace period"),
  (
    "add",
    "<name>: <command>",
    "Define a new process and start it",
  ),
  (
    "remove",
    "<targets>",
    "Stop and drop from the table entirely",
  ),
  ("ps", "", "Show the process table"),
  ("info", "<name>", "Show one process in detail"),
  ("focus", "<name|off>", "Show output from only one process"),
  ("mute", "<names>", "Hide output from processes"),
  ("unmute", "<names|all>", "Restore hidden output"),
  ("clear", "", "Clear the screen"),
  (
    "mode",
    "[restart|stop|ignore]",
    "Show or change the on-exit policy",
  ),
  (
    "help",
    "[command]",
    "Show this help, or detail for one command",
  ),
  ("quit", "", "Stop everything and exit"),
];

fn usage(name: &str) -> String {
  match HELP.iter().find(|(cmd, _, _)| *cmd == name) {
    Some((cmd, args, _)) if !args.is_empty() => format!("Usage: {} {}", cmd, args),
    _ => format!("Usage: {}", name),
  }
}

/// Parse a target list. Bare `all` selects every process.
fn parse_target(verb: &str, rest: &str) -> Result<Target, String> {
  if rest.is_empty() {
    return Err(usage(verb));
  }
  if rest.split_whitespace().eq(["all"]) {
    return Ok(Target::All);
  }
  Ok(Target::Names(
    rest.split_whitespace().map(str::to_string).collect(),
  ))
}

/// Parse a single required name argument.
fn parse_one(verb: &str, rest: &str) -> Result<String, String> {
  let mut words = rest.split_whitespace();
  let Some(name) = words.next() else {
    return Err(usage(verb));
  };
  if words.next().is_some() {
    return Err(format!("{} takes a single name", verb));
  }
  Ok(name.to_string())
}

fn parse_names(verb: &str, rest: &str) -> Result<Vec<String>, String> {
  if rest.is_empty() {
    return Err(usage(verb));
  }
  Ok(rest.split_whitespace().map(str::to_string).collect())
}

/// Parse a user input line into a routed command.
pub fn parse(input: &str) -> Result<Parsed, String> {
  let input = input.trim();
  if input.is_empty() {
    return Err(String::new());
  }

  let (cmd, rest) = match input.split_once(char::is_whitespace) {
    Some((c, r)) => (c, r.trim()),
    None => (input, ""),
  };

  match cmd {
    "start" => Ok(Parsed::Manager(Command::Start(parse_target(cmd, rest)?))),
    "stop" => Ok(Parsed::Manager(Command::Stop(parse_target(cmd, rest)?))),
    "restart" => Ok(Parsed::Manager(Command::Restart(parse_target(cmd, rest)?))),
    "kill" => Ok(Parsed::Manager(Command::Kill(parse_target(cmd, rest)?))),
    "remove" => Ok(Parsed::Manager(Command::Remove(parse_target(cmd, rest)?))),
    "add" => {
      let Some((name, command)) = rest.split_once(':') else {
        return Err(usage("add"));
      };
      let name = name.trim();
      let command = command.trim();
      if name.is_empty() || command.is_empty() {
        return Err(usage("add"));
      }
      if name.split_whitespace().count() > 1 {
        return Err("Process names cannot contain spaces".to_string());
      }
      Ok(Parsed::Manager(Command::Add(
        name.to_string(),
        command.to_string(),
      )))
    }
    "ps" => Ok(Parsed::Manager(Command::Ps)),
    "info" => Ok(Parsed::Manager(Command::Info(parse_one(cmd, rest)?))),
    "mode" if rest.is_empty() => Ok(Parsed::Manager(Command::Mode(None))),
    "mode" => match parse_one(cmd, rest)?.as_str() {
      "restart" => Ok(Parsed::Manager(Command::Mode(Some(OnExit::Restart)))),
      "stop" => Ok(Parsed::Manager(Command::Mode(Some(OnExit::Stop)))),
      "ignore" => Ok(Parsed::Manager(Command::Mode(Some(OnExit::Ignore)))),
      other => Err(format!("Unknown mode: {} ({})", other, usage("mode"))),
    },
    "quit" | "exit" => Ok(Parsed::Manager(Command::Quit)),
    "focus" => {
      let name = parse_one(cmd, rest)?;
      if name == "off" {
        Ok(Parsed::Display(DisplayCommand::Focus(None)))
      } else {
        Ok(Parsed::Display(DisplayCommand::Focus(Some(name))))
      }
    }
    "mute" => Ok(Parsed::Display(DisplayCommand::Mute(parse_names(
      cmd, rest,
    )?))),
    "unmute" => {
      let names = parse_names(cmd, rest)?;
      if names == ["all"] {
        Ok(Parsed::Display(DisplayCommand::UnmuteAll))
      } else {
        Ok(Parsed::Display(DisplayCommand::Unmute(names)))
      }
    }
    "clear" => Ok(Parsed::Display(DisplayCommand::Clear)),
    "help" => {
      if rest.is_empty() {
        Ok(Parsed::Help(None))
      } else {
        Ok(Parsed::Help(Some(parse_one("help", rest)?)))
      }
    }
    _ => match RENAMED.iter().find(|(old, _)| *old == cmd) {
      Some((_, new)) => Err(format!("Unknown command: {} — did you mean {}?", cmd, new)),
      None => Err(format!("Unknown command: {}", cmd)),
    },
  }
}

pub const PROMPT: &str = "procfile> ";

/// The prompt line as the renderer needs to see it. Shared with the stdout
/// consumer so a log line arriving mid-edit can repaint the prompt exactly,
/// cursor included.
#[derive(Clone, Debug, Default)]
pub struct PromptState {
  pub line: String,
  /// Cursor offset in characters (not bytes) from the start of `line`.
  pub cursor: usize,
}

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

/// Repaint the prompt line and place the terminal cursor at the edit position.
pub fn draw_prompt(state: &PromptState) {
  let mut out = stdout();
  let _ = execute!(
    out,
    cursor::MoveToColumn(0),
    term::Clear(term::ClearType::CurrentLine),
  );
  let _ = write!(out, "{}{}", PROMPT, state.line);
  let column = (PROMPT.chars().count() + state.cursor) as u16;
  let _ = execute!(out, cursor::MoveToColumn(column));
  let _ = out.flush();
}

/// Report a parse error as a log line rather than painting it on the prompt.
/// Anything drawn on the prompt line is erased by the next line of process
/// output, which on a busy Procfile can be under a second later.
fn emit_error(log_tx: &broadcast::Sender<LogEvent>, message: &str) {
  let _ = log_tx.send(LogEvent {
    process: "system".to_string(),
    color: Color::Red,
    line: message.to_string(),
    system: true,
    reply: true,
  });
}

/// Print transient lines above the prompt without routing them through the
/// log broadcast — completion candidates are a prompt affordance, not output
/// worth persisting or shipping to the web UI.
fn print_above_prompt(lines: &[String], state: &PromptState) {
  let mut out = stdout();
  let _ = execute!(
    out,
    cursor::MoveToColumn(0),
    term::Clear(term::ClearType::CurrentLine),
  );
  for line in lines {
    let _ = write!(out, "{}\r\n", line);
  }
  let _ = out.flush();
  draw_prompt(state);
}

fn reply(log_tx: &broadcast::Sender<LogEvent>, line: String) {
  let _ = log_tx.send(LogEvent {
    process: "system".to_string(),
    color: Color::White,
    line,
    system: true,
    reply: true,
  });
}

/// Emit the grouped command overview, or the detail for a single command.
fn emit_help(log_tx: &broadcast::Sender<LogEvent>, topic: Option<&str>) {
  if let Some(topic) = topic {
    match HELP.iter().find(|(cmd, _, _)| *cmd == topic) {
      Some((cmd, args, description)) => {
        reply(log_tx, format!("{} {}", cmd, args));
        reply(log_tx, format!("  {}", description));
      }
      None => reply(log_tx, format!("No such command: {}", topic)),
    }
    return;
  }

  let groups: &[(&str, &[&str])] = &[
    ("Lifecycle", &["start", "stop", "restart", "kill"]),
    ("Registry", &["add", "remove"]),
    ("Inspect", &["ps", "info"]),
    ("Output", &["focus", "mute", "unmute", "clear"]),
    ("Session", &["mode", "help", "quit"]),
  ];

  for (group, names) in groups {
    reply(log_tx, format!("{}:", group));
    for name in *names {
      let Some((cmd, args, description)) = HELP.iter().find(|(c, _, _)| c == name) else {
        continue;
      };
      let invocation = if args.is_empty() {
        cmd.to_string()
      } else {
        format!("{} {}", cmd, args)
      };
      reply(log_tx, format!("  {:<28} {}", invocation, description));
    }
  }
  reply(
    log_tx,
    "<targets> = one or more names, or \"all\"".to_string(),
  );
  reply(
    log_tx,
    "Tab completes · ↑↓ history · Ctrl+A/E/W/U/K edit".to_string(),
  );
}

/// Character bounds of the word the cursor sits in.
fn word_bounds(chars: &[char], cursor: usize) -> (usize, usize) {
  let start = chars[..cursor]
    .iter()
    .rposition(|c| c.is_whitespace())
    .map_or(0, |i| i + 1);
  (start, cursor)
}

/// Candidate completions for the word under the cursor, chosen by which
/// argument slot it occupies.
fn candidates(chars: &[char], word_start: usize, names: &[String]) -> Vec<String> {
  let leading: String = chars[..word_start].iter().collect();
  if leading.trim().is_empty() {
    return COMMANDS.iter().map(|c| c.to_string()).collect();
  }

  let verb = leading.split_whitespace().next().unwrap_or_default();
  match verb {
    "mode" => ["restart", "stop", "ignore"]
      .iter()
      .map(|c| c.to_string())
      .collect(),
    "help" => COMMANDS.iter().map(|c| c.to_string()).collect(),
    // `add` takes a free-form shell command; there is nothing to complete.
    "add" => Vec::new(),
    "focus" => names
      .iter()
      .cloned()
      .chain(std::iter::once("off".to_string()))
      .collect(),
    "info" | "mute" => names.to_vec(),
    "unmute" => names
      .iter()
      .cloned()
      .chain(std::iter::once("all".to_string()))
      .collect(),
    _ => names
      .iter()
      .cloned()
      .chain(std::iter::once("all".to_string()))
      .collect(),
  }
}

fn longest_common_prefix(items: &[String]) -> String {
  let Some(first) = items.first() else {
    return String::new();
  };
  let mut length = first.chars().count();
  for item in &items[1..] {
    length = first
      .chars()
      .zip(item.chars())
      .take(length)
      .take_while(|(a, b)| a == b)
      .count();
  }
  first.chars().take(length).collect()
}

/// Expand the word under the cursor. Returns candidates to display when the
/// prefix is ambiguous and cannot be extended further.
fn complete(chars: &mut Vec<char>, cursor: &mut usize, names: &[String]) -> Vec<String> {
  let (start, end) = word_bounds(chars, *cursor);
  let word: String = chars[start..end].iter().collect();

  let matches: Vec<String> = candidates(chars, start, names)
    .into_iter()
    .filter(|c| c.starts_with(&word))
    .collect();

  if matches.is_empty() {
    return Vec::new();
  }

  let expansion = if matches.len() == 1 {
    format!("{} ", matches[0])
  } else {
    longest_common_prefix(&matches)
  };

  if expansion.chars().count() > word.chars().count() {
    let replacement: Vec<char> = expansion.chars().collect();
    let added = replacement.len();
    chars.splice(start..end, replacement);
    *cursor = start + added;
    return Vec::new();
  }

  if matches.len() > 1 {
    matches
  } else {
    Vec::new()
  }
}

/// Run the input event loop. Reads keypresses, parses commands, sends events.
pub async fn run(
  input_tx: mpsc::UnboundedSender<InputEvent>,
  display_tx: mpsc::UnboundedSender<DisplayCommand>,
  prompt_tx: watch::Sender<PromptState>,
  names_rx: watch::Receiver<Vec<String>>,
  log_tx: broadcast::Sender<LogEvent>,
) {
  let mut chars: Vec<char> = Vec::new();
  let mut cursor: usize = 0;
  let mut history: Vec<String> = Vec::new();
  let mut history_pos: Option<usize> = None; // None = typing new input
  let mut saved_input = String::new(); // holds the in-progress line while browsing history

  // Publish the prompt so the stdout consumer can repaint it verbatim.
  macro_rules! publish {
    () => {{
      let state = PromptState {
        line: chars.iter().collect(),
        cursor,
      };
      let _ = prompt_tx.send(state.clone());
      state
    }};
  }

  draw_prompt(&publish!());

  loop {
    // Read next terminal event in a blocking thread
    let event = match tokio::task::spawn_blocking(crossterm::event::read).await {
      Ok(Ok(event)) => event,
      _ => break,
    };

    let Event::Key(KeyEvent {
      code,
      modifiers,
      kind,
      ..
    }) = event
    else {
      continue;
    };

    // crossterm may send both Press and Release events; only handle Press
    if kind != KeyEventKind::Press {
      continue;
    }

    let ctrl = modifiers.contains(KeyModifiers::CONTROL);

    match code {
      KeyCode::Char('c') if ctrl => {
        let _ = input_tx.send(InputEvent::CtrlC);
      }
      // Ctrl+D on an empty line is the conventional REPL exit; mid-line it is
      // a no-op rather than a surprise shutdown.
      KeyCode::Char('d') if ctrl && chars.is_empty() => {
        let _ = input_tx.send(InputEvent::Command(Command::Quit));
      }
      KeyCode::Char('l') if ctrl => {
        let _ = display_tx.send(DisplayCommand::Clear);
      }
      KeyCode::Char('a') if ctrl => {
        cursor = 0;
        draw_prompt(&publish!());
      }
      KeyCode::Char('e') if ctrl => {
        cursor = chars.len();
        draw_prompt(&publish!());
      }
      KeyCode::Char('u') if ctrl => {
        chars.drain(..cursor);
        cursor = 0;
        draw_prompt(&publish!());
      }
      KeyCode::Char('k') if ctrl => {
        chars.truncate(cursor);
        draw_prompt(&publish!());
      }
      KeyCode::Char('w') if ctrl => {
        // Skip trailing whitespace, then the word itself.
        let mut start = cursor;
        while start > 0 && chars[start - 1].is_whitespace() {
          start -= 1;
        }
        while start > 0 && !chars[start - 1].is_whitespace() {
          start -= 1;
        }
        chars.drain(start..cursor);
        cursor = start;
        draw_prompt(&publish!());
      }
      KeyCode::Char(c) if !ctrl && !modifiers.contains(KeyModifiers::ALT) => {
        chars.insert(cursor, c);
        cursor += 1;
        draw_prompt(&publish!());
      }
      KeyCode::Backspace => {
        if cursor > 0 {
          chars.remove(cursor - 1);
          cursor -= 1;
        }
        draw_prompt(&publish!());
      }
      KeyCode::Delete => {
        if cursor < chars.len() {
          chars.remove(cursor);
        }
        draw_prompt(&publish!());
      }
      KeyCode::Left => {
        cursor = cursor.saturating_sub(1);
        draw_prompt(&publish!());
      }
      KeyCode::Right => {
        if cursor < chars.len() {
          cursor += 1;
        }
        draw_prompt(&publish!());
      }
      KeyCode::Home => {
        cursor = 0;
        draw_prompt(&publish!());
      }
      KeyCode::End => {
        cursor = chars.len();
        draw_prompt(&publish!());
      }
      KeyCode::Tab => {
        let names = names_rx.borrow().clone();
        let listing = complete(&mut chars, &mut cursor, &names);
        let state = publish!();
        if listing.is_empty() {
          draw_prompt(&state);
        } else {
          print_above_prompt(&[listing.join("  ")], &state);
        }
      }
      KeyCode::Up => {
        if history.is_empty() {
          continue;
        }
        match history_pos {
          None => {
            saved_input = chars.iter().collect();
            history_pos = Some(history.len() - 1);
          }
          Some(pos) if pos > 0 => {
            history_pos = Some(pos - 1);
          }
          _ => continue,
        }
        chars = history[history_pos.unwrap()].chars().collect();
        cursor = chars.len();
        draw_prompt(&publish!());
      }
      KeyCode::Down => {
        let Some(pos) = history_pos else {
          continue;
        };
        if pos + 1 < history.len() {
          history_pos = Some(pos + 1);
          chars = history[pos + 1].chars().collect();
        } else {
          history_pos = None;
          chars = saved_input.chars().collect();
        }
        cursor = chars.len();
        draw_prompt(&publish!());
      }
      KeyCode::Enter => {
        let input: String = chars.iter().collect();
        chars.clear();
        cursor = 0;
        history_pos = None;
        saved_input.clear();
        let state = publish!();

        if !input.trim().is_empty() {
          // Don't add duplicates of the last entry
          if history.last().map(|h| h.as_str()) != Some(input.trim()) {
            history.push(input.trim().to_string());
          }
        }

        match parse(&input) {
          Ok(Parsed::Help(topic)) => {
            emit_help(&log_tx, topic.as_deref());
            draw_prompt(&state);
          }
          Ok(Parsed::Display(cmd)) => {
            let _ = display_tx.send(cmd);
            draw_prompt(&state);
          }
          Ok(Parsed::Manager(cmd)) => {
            let _ = input_tx.send(InputEvent::Command(cmd));
            draw_prompt(&state);
          }
          Err(msg) => {
            // An empty message means an empty line — nothing to report.
            if !msg.is_empty() {
              emit_error(&log_tx, &msg);
            }
            draw_prompt(&state);
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

  fn manager(input: &str) -> Command {
    match parse(input) {
      Ok(Parsed::Manager(cmd)) => cmd,
      other => panic!("expected a manager command, got {:?}", other),
    }
  }

  fn display(input: &str) -> DisplayCommand {
    match parse(input) {
      Ok(Parsed::Display(cmd)) => cmd,
      other => panic!("expected a display command, got {:?}", other),
    }
  }

  fn names(list: &[&str]) -> Target {
    Target::Names(list.iter().map(|n| n.to_string()).collect())
  }

  #[test]
  fn parse_single_target() {
    assert_eq!(manager("stop web"), Command::Stop(names(&["web"])));
  }

  #[test]
  fn parse_multiple_targets() {
    assert_eq!(
      manager("restart web worker css"),
      Command::Restart(names(&["web", "worker", "css"]))
    );
  }

  #[test]
  fn parse_all_target() {
    assert_eq!(manager("start all"), Command::Start(Target::All));
  }

  #[test]
  fn all_is_only_special_when_alone() {
    assert_eq!(
      manager("kill all web"),
      Command::Kill(names(&["all", "web"]))
    );
  }

  #[test]
  fn parse_add_with_colon() {
    assert_eq!(
      manager("add worker: bundle exec sidekiq"),
      Command::Add("worker".to_string(), "bundle exec sidekiq".to_string())
    );
  }

  #[test]
  fn add_preserves_colons_in_the_command() {
    assert_eq!(
      manager("add tunnel: ssh -L 5432:localhost:5432 db"),
      Command::Add(
        "tunnel".to_string(),
        "ssh -L 5432:localhost:5432 db".to_string()
      )
    );
  }

  #[test]
  fn add_requires_a_colon() {
    assert!(parse("add worker").is_err());
  }

  #[test]
  fn add_rejects_spaces_in_the_name() {
    assert!(parse("add my worker: sidekiq").is_err());
  }

  #[test]
  fn parse_ps() {
    assert_eq!(manager("ps"), Command::Ps);
  }

  #[test]
  fn parse_info() {
    assert_eq!(manager("info web"), Command::Info("web".to_string()));
  }

  #[test]
  fn info_rejects_multiple_names() {
    assert!(parse("info web worker").is_err());
  }

  #[test]
  fn parse_mode() {
    assert_eq!(manager("mode ignore"), Command::Mode(Some(OnExit::Ignore)));
  }

  #[test]
  fn bare_mode_queries_instead_of_setting() {
    assert_eq!(manager("mode"), Command::Mode(None));
  }

  #[test]
  fn parse_unknown_mode() {
    assert!(parse("mode sideways").is_err());
  }

  #[test]
  fn parse_quit_and_exit() {
    assert_eq!(manager("quit"), Command::Quit);
    assert_eq!(manager("exit"), Command::Quit);
  }

  #[test]
  fn parse_focus_and_off() {
    assert_eq!(
      display("focus web"),
      DisplayCommand::Focus(Some("web".to_string()))
    );
    assert_eq!(display("focus off"), DisplayCommand::Focus(None));
  }

  #[test]
  fn parse_mute_and_unmute() {
    assert_eq!(
      display("mute css worker"),
      DisplayCommand::Mute(vec!["css".to_string(), "worker".to_string()])
    );
    assert_eq!(display("unmute all"), DisplayCommand::UnmuteAll);
    assert_eq!(
      display("unmute css"),
      DisplayCommand::Unmute(vec!["css".to_string()])
    );
  }

  #[test]
  fn parse_clear() {
    assert_eq!(display("clear"), DisplayCommand::Clear);
  }

  #[test]
  fn parse_help_variants() {
    assert_eq!(parse("help"), Ok(Parsed::Help(None)));
    assert_eq!(
      parse("help stop"),
      Ok(Parsed::Help(Some("stop".to_string())))
    );
  }

  #[test]
  fn renamed_commands_suggest_the_replacement() {
    for (old, new) in RENAMED {
      let err = parse(old).unwrap_err();
      assert!(
        err.contains(new),
        "expected {:?} to point at {:?}, got {:?}",
        old,
        new,
        err
      );
    }
  }

  #[test]
  fn parse_unknown_command() {
    assert!(parse("foo").is_err());
  }

  #[test]
  fn empty_input_errors_without_a_message() {
    assert_eq!(parse(""), Err(String::new()));
    assert_eq!(parse("   "), Err(String::new()));
  }

  #[test]
  fn missing_target_errors() {
    assert!(parse("stop").is_err());
    assert!(parse("start").is_err());
  }

  #[test]
  fn every_command_has_a_help_entry() {
    for command in COMMANDS {
      assert!(
        HELP.iter().any(|(name, _, _)| name == command),
        "{} is missing a help entry",
        command
      );
    }
  }

  #[test]
  fn completion_expands_a_unique_command_prefix() {
    let mut chars: Vec<char> = "resta".chars().collect();
    let mut cursor = chars.len();
    let listing = complete(&mut chars, &mut cursor, &[]);
    assert_eq!(chars.iter().collect::<String>(), "restart ");
    assert_eq!(cursor, 8);
    assert!(listing.is_empty());
  }

  #[test]
  fn completion_extends_to_the_common_prefix_and_lists() {
    // "s" matches both "start" and "stop": extend to "st", then list.
    let mut chars: Vec<char> = "s".chars().collect();
    let mut cursor = chars.len();
    assert!(complete(&mut chars, &mut cursor, &[]).is_empty());
    assert_eq!(chars.iter().collect::<String>(), "st");

    let listing = complete(&mut chars, &mut cursor, &[]);
    assert!(listing.contains(&"start".to_string()));
    assert!(listing.contains(&"stop".to_string()));
  }

  #[test]
  fn completion_uses_process_names_in_argument_position() {
    let names = vec!["web".to_string(), "worker".to_string()];
    let mut chars: Vec<char> = "stop we".chars().collect();
    let mut cursor = chars.len();
    let listing = complete(&mut chars, &mut cursor, &names);
    assert_eq!(chars.iter().collect::<String>(), "stop web ");
    assert!(listing.is_empty());
  }

  #[test]
  fn completion_offers_modes_after_mode() {
    let mut chars: Vec<char> = "mode ig".chars().collect();
    let mut cursor = chars.len();
    complete(&mut chars, &mut cursor, &[]);
    assert_eq!(chars.iter().collect::<String>(), "mode ignore ");
  }

  #[test]
  fn completion_leaves_add_commands_alone() {
    let names = vec!["web".to_string()];
    let mut chars: Vec<char> = "add x: we".chars().collect();
    let mut cursor = chars.len();
    let listing = complete(&mut chars, &mut cursor, &names);
    assert_eq!(chars.iter().collect::<String>(), "add x: we");
    assert!(listing.is_empty());
  }
}
