use crate::ansi;
use crate::log_store::LogStore;
use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use rust_embed::Embed;
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

#[derive(Embed)]
#[folder = "frontend/dist"]
struct FrontendAssets;

const SSE_RETRY_MS: u64 = 2000;
/// Safe port range for deterministic hashing.
const PORT_RANGE_START: u16 = 2592;
const PORT_RANGE_END: u16 = 65535;
/// How many consecutive ports to try before giving up.
const PORT_RETRY_LIMIT: u16 = 64;

/// Build an SSE event for a log entry. Uses:
///   id:    — log index (enables Last-Event-ID reconnection)
///   event: — "log"
///   retry: — reconnect interval
///   data:  — JSON with process, color, line, html, timestamp, system
fn entry_to_sse(entry: &crate::log_store::LogEntry) -> Result<Event, Infallible> {
  let plain = ansi::strip_ansi_codes(&entry.line);
  let html = ansi::ansi_to_html(&entry.line);

  // Manual JSON to avoid serde dependency for this module
  let data = format!(
    "{{\"process\":{},\"color\":{},\"ts\":{},\"sys\":{},\"line\":{},\"html\":{}}}",
    json_str(&entry.process),
    json_str(&entry.color),
    json_str(&entry.timestamp),
    entry.system,
    json_str(&plain),
    json_str(&html),
  );

  Ok(
    Event::default()
      .event("log")
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

fn dir_name() -> String {
  std::env::current_dir()
    .ok()
    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
    .unwrap_or_else(|| "procfile".to_string())
}

async fn static_handler(uri: Uri) -> Response {
  let path = uri.path().trim_start_matches('/');

  // Serve index.html for root or any path without an extension (SPA fallback)
  let path = if path.is_empty() { "index.html" } else { path };

  match FrontendAssets::get(path) {
    Some(content) => {
      let mime = mime_guess::from_path(path).first_or_octet_stream();
      ([(header::CONTENT_TYPE, mime.as_ref())], content.data).into_response()
    }
    None => {
      // SPA fallback: serve index.html for unknown paths
      match FrontendAssets::get("index.html") {
        Some(content) => {
          ([(header::CONTENT_TYPE, "text/html")], content.data).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
      }
    }
  }
}

/// SSE handler: sends an `init` event first, then replays history (respecting
/// `Last-Event-ID`), then streams live entries.
async fn sse_handler(
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

/// Derive a deterministic port from the current working directory name.
pub fn port_for_cwd() -> u16 {
  let path = std::env::current_dir().unwrap_or_default();
  let name = path
    .file_name()
    .map(|n| n.to_string_lossy().into_owned())
    .unwrap_or_default();

  // Simple FNV-1a hash to spread across the port range
  let mut hash: u32 = 2166136261;
  for byte in name.as_bytes() {
    hash ^= *byte as u32;
    hash = hash.wrapping_mul(16777619);
  }

  let range = (PORT_RANGE_END - PORT_RANGE_START) as u32;
  PORT_RANGE_START + (hash % range) as u16
}

/// Start the web server, trying `port` first and falling back to consecutive ports.
pub async fn start(store: Arc<LogStore>, port: u16) {
  let app = Router::new()
    .route("/api/sse", get(sse_handler))
    .fallback(static_handler)
    .with_state(store);

  let mut current = port;
  let listener = loop {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], current));
    match tokio::net::TcpListener::bind(addr).await {
      Ok(listener) => break listener,
      Err(_) if current.saturating_add(1) <= port.saturating_add(PORT_RETRY_LIMIT) => {
        current = current.saturating_add(1);
      }
      Err(err) => {
        eprintln!(
          "Failed to bind web server after {} attempts: {}",
          PORT_RETRY_LIMIT, err
        );
        return;
      }
    }
  };

  let addr = listener.local_addr().expect("listener has local addr");
  eprintln!("Web UI available at http://{}", addr);

  if let Err(err) = axum::serve(listener, app).await {
    eprintln!("Web server error: {}", err);
  }
}
