use super::store::{LogEntry, LogStore};
use axum::extract::State;
use axum::response::sse::{Event, Sse};
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

const SSE_RETRY_MS: u64 = 2000;

fn dir_name() -> String {
  std::env::current_dir()
    .ok()
    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
    .unwrap_or_else(|| "procfile".to_string())
}

fn entry_to_sse(entry: &LogEntry) -> Result<Event, Infallible> {
  let data = format!(
    "{{\"process\":{},\"color\":{},\"ts\":{},\"sys\":{},\"line\":{}}}",
    json_str(&entry.process),
    json_str(&entry.color),
    json_str(&entry.timestamp),
    entry.system,
    json_str(&entry.line),
  );

  Ok(
    Event::default()
      .event("message")
      .id(entry.index.to_string())
      .retry(std::time::Duration::from_millis(SSE_RETRY_MS))
      .data(data),
  )
}

/// Escape a string for JSON.
fn json_str(s: &str) -> String {
  let mut out = String::with_capacity(s.len() + 2);
  out.push('"');
  for ch in s.chars() {
    match ch {
      '"' => out.push_str("\\\""),
      '\\' => out.push_str("\\\\"),
      '\n' => out.push_str("\\n"),
      '\r' => out.push_str("\\r"),
      '\t' => out.push_str("\\t"),
      c if c < '\x20' => {
        out.push_str(&format!("\\u{:04x}", c as u32));
      }
      c => out.push(c),
    }
  }
  out.push('"');
  out
}

/// SSE handler: sends an `init` event first, then replays history (respecting
/// `Last-Event-ID`), then streams live entries as `message` events.
pub(super) async fn handler(
  headers: axum::http::HeaderMap,
  State(store): State<Arc<LogStore>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
  let last_id: Option<usize> = headers
    .get("Last-Event-ID")
    .and_then(|v| v.to_str().ok())
    .and_then(|s| s.parse().ok());

  // Subscribe BEFORE reading existing entries to avoid missing anything.
  let rx = store.subscribe();
  let existing = store.get_all();

  // Init event (only on fresh connections, not reconnects)
  let init_events: Vec<Result<Event, Infallible>> = if last_id.is_none() {
    vec![Ok(
      Event::default()
        .event("init")
        .retry(std::time::Duration::from_millis(SSE_RETRY_MS))
        .data(format!("{{\"name\":{}}}", json_str(&dir_name()))),
    )]
  } else {
    vec![]
  };
  let init_stream = tokio_stream::iter(init_events);

  // Only replay entries after last_id (or all if fresh connection).
  let replay: Vec<_> = existing
    .iter()
    .filter(|e| match last_id {
      Some(id) => e.index > id,
      None => true,
    })
    .map(entry_to_sse)
    .collect();

  let last_index = existing.last().map(|e| e.index);
  let history = tokio_stream::iter(replay);

  // Live stream, skipping entries already covered by replay.
  let live = BroadcastStream::new(rx).filter_map(move |result| {
    result.ok().and_then(|entry| {
      if let Some(last) = last_index {
        if entry.index <= last {
          return None;
        }
      }
      Some(entry_to_sse(&entry))
    })
  });

  Sse::new(init_stream.chain(history).chain(live))
}
