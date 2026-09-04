use crate::core::manager::{OnExit, Snapshot};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use tokio::sync::{mpsc, watch};

/// The set of processes a lifecycle command applies to.
#[derive(Debug, PartialEq, Clone)]
pub enum Target {
  All,
  Names(Vec<String>),
}

/// A command that mutates or inspects process state. Handled by the manager.
#[derive(Debug, PartialEq, Clone)]
pub enum Command {
  Start(Target),
  Stop(Target),
  Restart(Target),
  Kill(Target),
  Add(String, String),
  Remove(Target),
  Mode(OnExit),
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

/// Read terminal events on a single dedicated thread and forward them.
///
/// `crossterm::event::read()` blocks and must never be called from two
/// places at once, but both the base loop and the modal need to react to
/// keypresses. Centralizing the read here lets both consume from a channel
/// instead of racing blocking calls against each other.
pub async fn read_events(tx: mpsc::UnboundedSender<Event>) {
  while let Ok(Ok(event)) = tokio::task::spawn_blocking(crossterm::event::read).await {
    if tx.send(event).is_err() {
      break;
    }
  }
}

/// Run the base key-listening loop.
///
/// Outside the modal, the terminal is just scrolling log output — no prompt
/// line to maintain. Only a couple of global hotkeys are recognized here;
/// everything else (including all process actions) lives behind the `G`
/// modal so a stray keypress can never be mistaken for a command.
pub async fn run(
  input_tx: mpsc::UnboundedSender<InputEvent>,
  display_tx: mpsc::UnboundedSender<DisplayCommand>,
  modal_active: watch::Sender<bool>,
  mut snapshot_rx: watch::Receiver<Snapshot>,
  mut key_rx: mpsc::UnboundedReceiver<Event>,
) {
  loop {
    let Some(event) = key_rx.recv().await else {
      break;
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
      KeyCode::Char('l') if ctrl => {
        let _ = display_tx.send(DisplayCommand::Clear);
      }
      KeyCode::Char('g' | 'G') if !ctrl => {
        let _ = modal_active.send(true);
        crate::tui::run(&input_tx, &display_tx, &mut snapshot_rx, &mut key_rx).await;
        let _ = modal_active.send(false);
      }
      _ => {}
    }
  }
}
