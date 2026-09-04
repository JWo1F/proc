mod app;
mod view;

use crate::core::manager::Snapshot;
use crate::input::{DisplayCommand, InputEvent};
use app::App;
use crossterm::event::{Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::stdout;
use tokio::sync::{mpsc, watch};

/// Enter the alternate screen, drive the modal until it closes, then restore
/// the scrolling log view exactly as it was.
pub async fn run(
  input_tx: &mpsc::UnboundedSender<InputEvent>,
  display_tx: &mpsc::UnboundedSender<DisplayCommand>,
  snapshot_rx: &mut watch::Receiver<Snapshot>,
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
  let _ = terminal.clear();

  let mut app = App::new(snapshot_rx.borrow().clone());

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
