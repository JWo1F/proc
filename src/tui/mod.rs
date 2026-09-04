mod app;
mod view;

use crate::core::manager::Snapshot;
use crate::core::resources::ResourceMap;
use crate::input::{DisplayCommand, InputEvent};
use app::App;
use crossterm::event::{Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
use std::io::stdout;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// How often to redraw even without a new event — keeps uptime, and the
/// resource graphs, visibly ticking while a screen is just sitting open.
const TICK_INTERVAL: Duration = Duration::from_secs(1);

/// Enter the alternate screen, drive the modal until it closes, then restore
/// the scrolling log view exactly as it was.
pub async fn run(
  input_tx: &mpsc::UnboundedSender<InputEvent>,
  display_tx: &mpsc::UnboundedSender<DisplayCommand>,
  snapshot_rx: &mut watch::Receiver<Snapshot>,
  resources_rx: &mut watch::Receiver<ResourceMap>,
  key_rx: &mut mpsc::UnboundedReceiver<Event>,
) {
  if execute!(stdout(), EnterAlternateScreen).is_err() {
    return;
  }

  let mut terminal = match Terminal::new(CrosstermBackend::new(stdout())) {
    Ok(terminal) => terminal,
    Err(_) => {
      let _ = execute!(stdout(), LeaveAlternateScreen);
      return;
    }
  };
  let _ = terminal.hide_cursor();
  // `Terminal::clear` first asks the terminal where the cursor is, and that
  // query blocks for a full two seconds here: the reply arrives on stdin,
  // where the dedicated event-reader thread swallows it, so crossterm waits
  // out its timeout on every open. Clear through the backend instead — the
  // alternate screen starts blank and this Terminal's buffers are empty, so
  // the buffer reset `Terminal::clear` would also do is a no-op.
  let _ = terminal.backend_mut().clear();

  let mut app = App::new(snapshot_rx.borrow().clone(), resources_rx.borrow().clone());
  let mut tick = tokio::time::interval(TICK_INTERVAL);
  tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

  while !app.should_close {
    if terminal.draw(|frame| view::render(frame, &app)).is_err() {
      break;
    }

    tokio::select! {
      changed = snapshot_rx.changed() => {
        match changed {
          Ok(()) => app.snapshot = snapshot_rx.borrow_and_update().clone(),
          Err(_) => break,
        }
      }
      changed = resources_rx.changed() => {
        match changed {
          Ok(()) => app.resources = resources_rx.borrow_and_update().clone(),
          Err(_) => break,
        }
      }
      _ = tick.tick() => {}
      event = key_rx.recv() => {
        match event {
          Some(Event::Key(key)) if key.kind == KeyEventKind::Press => {
            app.handle_key(key, input_tx, display_tx);
          }
          Some(_) => {}
          None => break,
        }
      }
    }
  }

  let _ = terminal.show_cursor();
  let _ = execute!(stdout(), LeaveAlternateScreen);
}
