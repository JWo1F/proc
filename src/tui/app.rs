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
  ('o', "Run mode"),
  ('f', "Focus"),
  ('m', "Mute"),
  ('u', "Unmute"),
  ('x', "Remove"),
  ('b', "Back"),
];

/// Per-process counterpart to `MODE_ITEMS`: the same policies, worded for one
/// process, plus a way to hand it back to the session mode.
pub const PROCESS_MODE_ITEMS: &[Item] = &[
  ('r', "Restart — respawn it on exit"),
  ('s', "Stop — end the run on exit"),
  ('o', "Once — leave it down on exit"),
  ('f', "Follow the session mode"),
  ('b', "Back"),
];

/// The run mode each `PROCESS_MODE_ITEMS` row selects.
pub const PROCESS_MODES: &[Option<OnExit>] = &[
  Some(OnExit::Restart),
  Some(OnExit::Stop),
  Some(OnExit::Ignore),
  None,
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
  if key
    .modifiers
    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
  {
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
  ProcessMode {
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
  ProcessMode {
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
      Screen::Main { selected } => Peek::Main {
        selected: *selected,
      },
      Screen::Processes { selected } => Peek::Processes {
        selected: *selected,
      },
      Screen::ProcessActions { name, selected } => Peek::ProcessActions {
        name: name.clone(),
        selected: *selected,
      },
      Screen::ProcessMode { name, selected } => Peek::ProcessMode {
        name: name.clone(),
        selected: *selected,
      },
      Screen::AllActions { selected } => Peek::AllActions {
        selected: *selected,
      },
      Screen::ModeMenu { selected } => Peek::ModeMenu {
        selected: *selected,
      },
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
      Some(Screen::ProcessMode { selected, .. }) => Some(*selected),
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
      Some(Screen::ProcessMode { selected, .. }) => *selected = new_selected,
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

    let plain =
      !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT);

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
        self.push(Screen::ProcessMode { name, selected: 0 });
        return;
      }
      6 => {
        let _ = display_tx.send(DisplayCommand::Focus(Some(name)));
      }
      7 => {
        let _ = display_tx.send(DisplayCommand::Mute(vec![name]));
      }
      8 => {
        let _ = display_tx.send(DisplayCommand::Unmute(vec![name]));
      }
      9 => send_command(input_tx, Command::Remove(target())),
      10 => {
        self.pop();
        return;
      }
      _ => return,
    }
    self.should_close = true;
  }

  /// Pin this one process to its own exit policy. The session mode and every
  /// other process are left alone.
  fn activate_process_mode(
    &mut self,
    name: String,
    index: usize,
    input_tx: &UnboundedSender<InputEvent>,
  ) {
    let Some(mode) = PROCESS_MODES.get(index) else {
      // The only row past the modes is Back.
      if index == PROCESS_MODE_ITEMS.len() - 1 {
        self.pop();
      }
      return;
    };

    send_command(
      input_tx,
      Command::ProcessMode(Target::Names(vec![name]), *mode),
    );
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

      Peek::ProcessMode { name, selected } => {
        if self.nav_current(PROCESS_MODE_ITEMS.len(), key.code) {
          return;
        }
        if let Some(index) = hotkey_index(PROCESS_MODE_ITEMS, key) {
          self.activate_process_mode(name, index, input_tx);
          return;
        }
        match key.code {
          KeyCode::Esc => self.pop(),
          KeyCode::Enter => self.activate_process_mode(name, selected, input_tx),
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
            let name = name.trim().to_string();

            // Muting is the stdout consumer's business, not the manager's, so
            // a `muted` flag typed into the name has to be routed separately.
            // An unparseable name is left for the manager to report.
            if let Ok((name, flags)) = crate::core::procfile::parse_name(&name)
              && flags.muted
            {
              let _ = display_tx.send(DisplayCommand::Mute(vec![name.to_string()]));
            }

            send_command(input_tx, Command::Add(name, command.trim().to_string()));
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::core::manager::ProcessSnapshot;
  use crate::core::procfile::Flags;
  use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

  /// An `App` wired to real channels, so a test can press keys and read back
  /// whatever the modal sent to the manager.
  struct Harness {
    app: App,
    input_tx: UnboundedSender<InputEvent>,
    input_rx: UnboundedReceiver<InputEvent>,
    display_tx: UnboundedSender<DisplayCommand>,
    display_rx: UnboundedReceiver<DisplayCommand>,
  }

  impl Harness {
    fn new(processes: &[(&str, Flags)]) -> Self {
      let snapshot = Snapshot {
        mode: OnExit::Stop,
        processes: processes
          .iter()
          .map(|(name, flags)| ProcessSnapshot {
            name: name.to_string(),
            command: "echo hi".to_string(),
            status: "running",
            pid: None,
            uptime: None,
            restarts: 0,
            last_exit: None,
            flags: *flags,
          })
          .collect(),
      };
      let (input_tx, input_rx) = unbounded_channel();
      let (display_tx, display_rx) = unbounded_channel();
      Self {
        app: App::new(snapshot, ResourceMap::new()),
        input_tx,
        input_rx,
        display_tx,
        display_rx,
      }
    }

    fn press(&mut self, code: KeyCode) {
      let key = KeyEvent::new(code, KeyModifiers::NONE);
      self.app.handle_key(key, &self.input_tx, &self.display_tx);
    }

    fn type_keys(&mut self, keys: &str) {
      for c in keys.chars() {
        self.press(KeyCode::Char(c));
      }
    }

    fn command(&mut self) -> Option<Command> {
      match self.input_rx.try_recv() {
        Ok(InputEvent::Command(cmd)) => Some(cmd),
        _ => None,
      }
    }

    fn display(&mut self) -> Option<DisplayCommand> {
      self.display_rx.try_recv().ok()
    }
  }

  fn one_process() -> Vec<(&'static str, Flags)> {
    vec![("web", Flags::default())]
  }

  fn target() -> Target {
    Target::Names(vec!["web".to_string()])
  }

  /// The action list and its dispatch table are matched by index, so every
  /// row has to be checked — inserting one in the middle must not silently
  /// shift `Remove` onto someone else's key.
  #[test]
  fn every_process_action_hotkey_dispatches_its_own_row() {
    let expected: &[(char, Option<Command>)] = &[
      ('s', Some(Command::Start(target()))),
      ('t', Some(Command::Stop(target()))),
      ('r', Some(Command::Restart(target()))),
      ('k', Some(Command::Kill(target()))),
      ('x', Some(Command::Remove(target()))),
    ];

    for (hotkey, command) in expected {
      let mut h = Harness::new(&one_process());
      h.type_keys("p1");
      h.press(KeyCode::Char(*hotkey));
      assert_eq!(h.command(), *command, "hotkey {:?}", hotkey);
      assert!(
        h.app.should_close,
        "hotkey {:?} should close the modal",
        hotkey
      );
    }
  }

  #[test]
  fn display_only_actions_never_reach_the_manager() {
    let expected: &[(char, DisplayCommand)] = &[
      ('f', DisplayCommand::Focus(Some("web".to_string()))),
      ('m', DisplayCommand::Mute(vec!["web".to_string()])),
      ('u', DisplayCommand::Unmute(vec!["web".to_string()])),
    ];

    for (hotkey, display) in expected {
      let mut h = Harness::new(&one_process());
      h.type_keys("p1");
      h.press(KeyCode::Char(*hotkey));
      assert_eq!(h.display(), Some(display.clone()), "hotkey {:?}", hotkey);
      assert_eq!(h.command(), None, "hotkey {:?}", hotkey);
    }
  }

  #[test]
  fn info_and_back_stay_inside_the_modal() {
    let mut h = Harness::new(&one_process());
    h.type_keys("p1i");
    assert!(matches!(h.app.stack.last(), Some(Screen::Info { .. })));
    assert!(!h.app.should_close);

    h.press(KeyCode::Esc);
    h.press(KeyCode::Char('b'));
    assert!(matches!(h.app.stack.last(), Some(Screen::Processes { .. })));
  }

  #[test]
  fn the_run_mode_screen_pins_one_process_to_its_own_policy() {
    let expected: &[(char, Option<OnExit>)] = &[
      ('r', Some(OnExit::Restart)),
      ('s', Some(OnExit::Stop)),
      ('o', Some(OnExit::Ignore)),
      ('f', None),
    ];

    for (hotkey, mode) in expected {
      let mut h = Harness::new(&one_process());
      h.type_keys("p1o");
      assert!(matches!(
        h.app.stack.last(),
        Some(Screen::ProcessMode { .. })
      ));

      h.press(KeyCode::Char(*hotkey));
      assert_eq!(
        h.command(),
        Some(Command::ProcessMode(target(), *mode)),
        "hotkey {:?}",
        hotkey
      );
    }
  }

  #[test]
  fn back_out_of_the_run_mode_screen_sends_nothing() {
    let mut h = Harness::new(&one_process());
    h.type_keys("p1ob");

    assert_eq!(h.command(), None);
    assert!(!h.app.should_close);
    assert!(matches!(
      h.app.stack.last(),
      Some(Screen::ProcessActions { .. })
    ));
  }

  #[test]
  fn starting_an_optional_process_targets_it_by_name() {
    let optional = Flags {
      optional: true,
      ..Flags::default()
    };
    let mut h = Harness::new(&[("web", Flags::default()), ("seed", optional)]);

    // Row 2 is the optional process; Start is how the menu brings it in.
    h.type_keys("p2s");

    assert_eq!(
      h.command(),
      Some(Command::Start(Target::Names(vec!["seed".to_string()])))
    );
  }
}
