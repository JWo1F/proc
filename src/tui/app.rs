use crate::core::manager::{OnExit, Snapshot};
use crate::core::resources::ResourceMap;
use crate::input::{Command, DisplayCommand, InputEvent, Target};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc::UnboundedSender;

/// A menu item paired with the single key that activates it directly,
/// without going through arrow-key navigation first.
pub type Item = (char, &'static str);

pub const MAIN_ITEMS: &[Item] = &[
  ('p', "Processes"),
  ('a', "Add process"),
  ('l', "All processes"),
  ('m', "Mode"),
  ('d', "Dashboard"),
  ('s', "Ps — process table"),
  ('q', "Quit"),
];

pub const PROCESS_ACTION_ITEMS: &[Item] = &[
  ('s', "Start"),
  ('t', "Stop"),
  ('r', "Restart"),
  ('k', "Kill"),
  ('i', "Info"),
  ('f', "Focus"),
  ('m', "Mute"),
  ('u', "Unmute"),
  ('x', "Remove"),
  ('b', "Back"),
];

pub const ALL_ACTION_ITEMS: &[Item] = &[
  ('s', "Start all"),
  ('t', "Stop all"),
  ('r', "Restart all"),
  ('k', "Kill all"),
  ('u', "Unmute all"),
  ('f', "Focus off"),
  ('x', "Remove all"),
  ('b', "Back"),
];

pub const MODE_ITEMS: &[Item] = &[
  ('r', "Restart"),
  ('s', "Stop"),
  ('i', "Ignore"),
  ('b', "Back"),
];

/// Index into `Item` by its hotkey, ignoring case and any modifier keys that
/// would make it a different shortcut (Ctrl+P, etc).
fn hotkey_index(items: &[Item], key: KeyEvent) -> Option<usize> {
  if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
    return None;
  }
  let KeyCode::Char(c) = key.code else {
    return None;
  };
  let c = c.to_ascii_lowercase();
  items.iter().position(|(hotkey, _)| *hotkey == c)
}

#[derive(Clone, Copy, PartialEq)]
pub enum AddField {
  Name,
  Command,
}

/// One level of the modal's navigation stack.
pub enum Screen {
  Main {
    selected: usize,
  },
  Processes {
    selected: usize,
  },
  ProcessActions {
    name: String,
    selected: usize,
  },
  AllActions {
    selected: usize,
  },
  ModeMenu {
    selected: usize,
  },
  AddProcess {
    name: String,
    command: String,
    field: AddField,
  },
  Info {
    name: String,
  },
  Ps,
  Dashboard,
  ConfirmQuit,
}

/// An owned copy of the current screen's state, detached from `self` so
/// handling it can freely call back into other `&mut self` methods without
/// fighting the borrow checker over `self.stack`.
enum Peek {
  Main { selected: usize },
  Processes { selected: usize },
  ProcessActions { name: String, selected: usize },
  AllActions { selected: usize },
  ModeMenu { selected: usize },
  AddProcess {
    name: String,
    command: String,
    field: AddField,
  },
  Info,
  Ps,
  Dashboard,
  ConfirmQuit,
}

pub struct App {
  pub stack: Vec<Screen>,
  pub snapshot: Snapshot,
  pub resources: ResourceMap,
  pub should_close: bool,
}

/// Move a `selected` index up or down within `len` items, clamped at the ends.
fn nav_delta(selected: usize, len: usize, code: KeyCode) -> Option<usize> {
  match code {
    KeyCode::Up => Some(selected.saturating_sub(1)),
    KeyCode::Down => Some((selected + 1).min(len.saturating_sub(1))),
    _ => None,
  }
}

impl App {
  pub fn new(snapshot: Snapshot, resources: ResourceMap) -> Self {
    Self {
      stack: vec![Screen::Main { selected: 0 }],
      snapshot,
      resources,
      should_close: false,
    }
  }

  fn peek(&self) -> Peek {
    match self.stack.last().expect("stack is never empty") {
      Screen::Main { selected } => Peek::Main { selected: *selected },
      Screen::Processes { selected } => Peek::Processes { selected: *selected },
      Screen::ProcessActions { name, selected } => Peek::ProcessActions {
        name: name.clone(),
        selected: *selected,
      },
      Screen::AllActions { selected } => Peek::AllActions { selected: *selected },
      Screen::ModeMenu { selected } => Peek::ModeMenu { selected: *selected },
      Screen::AddProcess {
        name,
        command,
        field,
      } => Peek::AddProcess {
        name: name.clone(),
        command: command.clone(),
        field: *field,
      },
      Screen::Info { .. } => Peek::Info,
      Screen::Ps => Peek::Ps,
      Screen::Dashboard => Peek::Dashboard,
      Screen::ConfirmQuit => Peek::ConfirmQuit,
    }
  }

  fn push(&mut self, screen: Screen) {
    self.stack.push(screen);
  }

  fn pop(&mut self) {
    if self.stack.len() > 1 {
      self.stack.pop();
    } else {
      self.should_close = true;
    }
  }

  /// Apply a navigation key to whichever list screen is on top. Returns
  /// `true` if the key was consumed (caller should stop processing it).
  fn nav_current(&mut self, len: usize, code: KeyCode) -> bool {
    let Some(selected) = (match self.stack.last() {
      Some(Screen::Main { selected }) => Some(*selected),
      Some(Screen::Processes { selected }) => Some(*selected),
      Some(Screen::ProcessActions { selected, .. }) => Some(*selected),
      Some(Screen::AllActions { selected }) => Some(*selected),
      Some(Screen::ModeMenu { selected }) => Some(*selected),
      _ => None,
    }) else {
      return false;
    };

    let Some(new_selected) = nav_delta(selected, len, code) else {
      return false;
    };

    match self.stack.last_mut() {
      Some(Screen::Main { selected }) => *selected = new_selected,
      Some(Screen::Processes { selected }) => *selected = new_selected,
      Some(Screen::ProcessActions { selected, .. }) => *selected = new_selected,
      Some(Screen::AllActions { selected }) => *selected = new_selected,
      Some(Screen::ModeMenu { selected }) => *selected = new_selected,
      _ => {}
    }
    true
  }

  fn process_name(&self, index: usize) -> Option<String> {
    self.snapshot.processes.get(index).map(|p| p.name.clone())
  }

  fn set_add_field(&mut self, field: AddField) {
    if let Some(Screen::AddProcess { field: f, .. }) = self.stack.last_mut() {
      *f = field;
    }
  }

  fn edit_add_process(&mut self, key: KeyEvent) {
    let Some(Screen::AddProcess {
      name,
      command,
      field,
    }) = self.stack.last_mut()
    else {
      return;
    };

    let plain = !key.modifiers.contains(KeyModifiers::CONTROL)
      && !key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
      KeyCode::Tab | KeyCode::Down | KeyCode::Up => {
        *field = match field {
          AddField::Name => AddField::Command,
          AddField::Command => AddField::Name,
        };
      }
      KeyCode::Backspace => {
        match field {
          AddField::Name => name.pop(),
          AddField::Command => command.pop(),
        };
      }
      KeyCode::Char(c) if plain => match field {
        AddField::Name => name.push(c),
        AddField::Command => command.push(c),
      },
      _ => {}
    }
  }

  fn activate_main(&mut self, index: usize) {
    match index {
      0 => self.push(Screen::Processes { selected: 0 }),
      1 => self.push(Screen::AddProcess {
        name: String::new(),
        command: String::new(),
        field: AddField::Name,
      }),
      2 => self.push(Screen::AllActions { selected: 0 }),
      3 => self.push(Screen::ModeMenu { selected: 0 }),
      4 => self.push(Screen::Dashboard),
      5 => self.push(Screen::Ps),
      6 => self.push(Screen::ConfirmQuit),
      _ => {}
    }
  }

  fn activate_process_actions(
    &mut self,
    name: String,
    index: usize,
    input_tx: &UnboundedSender<InputEvent>,
    display_tx: &UnboundedSender<DisplayCommand>,
  ) {
    let target = || Target::Names(vec![name.clone()]);
    match index {
      0 => send_command(input_tx, Command::Start(target())),
      1 => send_command(input_tx, Command::Stop(target())),
      2 => send_command(input_tx, Command::Restart(target())),
      3 => send_command(input_tx, Command::Kill(target())),
      4 => {
        self.push(Screen::Info { name });
        return;
      }
      5 => {
        let _ = display_tx.send(DisplayCommand::Focus(Some(name)));
      }
      6 => {
        let _ = display_tx.send(DisplayCommand::Mute(vec![name]));
      }
      7 => {
        let _ = display_tx.send(DisplayCommand::Unmute(vec![name]));
      }
      8 => send_command(input_tx, Command::Remove(target())),
      9 => {
        self.pop();
        return;
      }
      _ => return,
    }
    self.should_close = true;
  }

  fn activate_all_actions(
    &mut self,
    index: usize,
    input_tx: &UnboundedSender<InputEvent>,
    display_tx: &UnboundedSender<DisplayCommand>,
  ) {
    match index {
      0 => send_command(input_tx, Command::Start(Target::All)),
      1 => send_command(input_tx, Command::Stop(Target::All)),
      2 => send_command(input_tx, Command::Restart(Target::All)),
      3 => send_command(input_tx, Command::Kill(Target::All)),
      4 => {
        let _ = display_tx.send(DisplayCommand::UnmuteAll);
      }
      5 => {
        let _ = display_tx.send(DisplayCommand::Focus(None));
      }
      6 => send_command(input_tx, Command::Remove(Target::All)),
      7 => {
        self.pop();
        return;
      }
      _ => return,
    }
    self.should_close = true;
  }

  fn activate_mode_menu(&mut self, index: usize, input_tx: &UnboundedSender<InputEvent>) {
    match index {
      0 => send_command(input_tx, Command::Mode(OnExit::Restart)),
      1 => send_command(input_tx, Command::Mode(OnExit::Stop)),
      2 => send_command(input_tx, Command::Mode(OnExit::Ignore)),
      3 => {
        self.pop();
        return;
      }
      _ => return,
    }
    self.should_close = true;
  }

  pub fn handle_key(
    &mut self,
    key: KeyEvent,
    input_tx: &UnboundedSender<InputEvent>,
    display_tx: &UnboundedSender<DisplayCommand>,
  ) {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
      let _ = input_tx.send(InputEvent::CtrlC);
      self.should_close = true;
      return;
    }

    match self.peek() {
      Peek::Main { selected } => {
        if self.nav_current(MAIN_ITEMS.len(), key.code) {
          return;
        }
        if let Some(index) = hotkey_index(MAIN_ITEMS, key) {
          self.activate_main(index);
          return;
        }
        match key.code {
          KeyCode::Esc => self.pop(),
          KeyCode::Enter => self.activate_main(selected),
          _ => {}
        }
      }

      Peek::Processes { selected } => {
        let len = self.snapshot.processes.len();
        if len > 0 && self.nav_current(len, key.code) {
          return;
        }
        // 1-9 jump straight to that row, skipping the highlight step.
        if let KeyCode::Char(c @ '1'..='9') = key.code {
          let index = c.to_digit(10).expect("'1'..='9' always parses") as usize - 1;
          if let Some(name) = self.process_name(index) {
            self.push(Screen::ProcessActions { name, selected: 0 });
          }
          return;
        }
        match key.code {
          KeyCode::Esc => self.pop(),
          KeyCode::Enter => {
            if let Some(name) = self.process_name(selected) {
              self.push(Screen::ProcessActions { name, selected: 0 });
            }
          }
          _ => {}
        }
      }

      Peek::ProcessActions { name, selected } => {
        if self.nav_current(PROCESS_ACTION_ITEMS.len(), key.code) {
          return;
        }
        if let Some(index) = hotkey_index(PROCESS_ACTION_ITEMS, key) {
          self.activate_process_actions(name, index, input_tx, display_tx);
          return;
        }
        match key.code {
          KeyCode::Esc => self.pop(),
          KeyCode::Enter => self.activate_process_actions(name, selected, input_tx, display_tx),
          _ => {}
        }
      }

      Peek::AllActions { selected } => {
        if self.nav_current(ALL_ACTION_ITEMS.len(), key.code) {
          return;
        }
        if let Some(index) = hotkey_index(ALL_ACTION_ITEMS, key) {
          self.activate_all_actions(index, input_tx, display_tx);
          return;
        }
        match key.code {
          KeyCode::Esc => self.pop(),
          KeyCode::Enter => self.activate_all_actions(selected, input_tx, display_tx),
          _ => {}
        }
      }

      Peek::ModeMenu { selected } => {
        if self.nav_current(MODE_ITEMS.len(), key.code) {
          return;
        }
        if let Some(index) = hotkey_index(MODE_ITEMS, key) {
          self.activate_mode_menu(index, input_tx);
          return;
        }
        match key.code {
          KeyCode::Esc => self.pop(),
          KeyCode::Enter => self.activate_mode_menu(selected, input_tx),
          _ => {}
        }
      }

      Peek::AddProcess {
        name,
        command,
        field,
      } => match key.code {
        KeyCode::Esc => self.pop(),
        KeyCode::Enter => {
          if field == AddField::Name {
            self.set_add_field(AddField::Command);
          } else if !name.trim().is_empty() && !command.trim().is_empty() {
            send_command(
              input_tx,
              Command::Add(name.trim().to_string(), command.trim().to_string()),
            );
            self.should_close = true;
          }
        }
        _ => self.edit_add_process(key),
      },

      Peek::Info => {
        if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
          self.pop();
        }
      }

      Peek::Ps => {
        if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
          self.pop();
        }
      }

      Peek::Dashboard => {
        if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
          self.pop();
        }
      }

      Peek::ConfirmQuit => match key.code {
        KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
          send_command(input_tx, Command::Quit);
          self.should_close = true;
        }
        KeyCode::Char('n' | 'N') | KeyCode::Esc => self.pop(),
        _ => {}
      },
    }
  }
}

fn send_command(input_tx: &UnboundedSender<InputEvent>, cmd: Command) {
  let _ = input_tx.send(InputEvent::Command(cmd));
}
