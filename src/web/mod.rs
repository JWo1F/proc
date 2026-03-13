mod assets;
mod sse;
mod store;

use crate::core::LogEvent;
use axum::routing::get;
use axum::Router;
use colored::Color;
use axum::http::{HeaderName, HeaderValue};
use std::sync::Arc;
use store::LogStore;
use tokio::sync::broadcast;
use tower_http::set_header::SetResponseHeaderLayer;

/// Safe port range for deterministic hashing.
const PORT_RANGE_START: u16 = 2592;
const PORT_RANGE_END: u16 = 65535;
/// How many consecutive ports to try before giving up.
const PORT_RETRY_LIMIT: u16 = 64;

fn color_to_css(color: &Color) -> String {
  match color {
    Color::TrueColor { r, g, b } => format!("rgb({},{},{})", r, g, b),
    Color::Black => "#000".to_string(),
    Color::Red => "#cd3131".to_string(),
    Color::Green => "#0dbc79".to_string(),
    Color::Yellow => "#e5e510".to_string(),
    Color::Blue => "#2472c8".to_string(),
    Color::Magenta => "#bc3fbc".to_string(),
    Color::Cyan => "#11a8cd".to_string(),
    Color::White => "#e5e5e5".to_string(),
    _ => "inherit".to_string(),
  }
}

/// Reads LogEvents from the core broadcast channel and feeds the web LogStore.
async fn log_consumer(mut rx: broadcast::Receiver<LogEvent>, store: Arc<LogStore>) {
  loop {
    match rx.recv().await {
      Ok(event) => {
        let now = chrono::Utc::now();
        let ts = now.timestamp() as f64 + now.timestamp_subsec_millis() as f64 / 1000.0;
        let css_color = color_to_css(&event.color);
        store.push(&event.process, &css_color, &event.line, ts, event.system);
      }
      Err(broadcast::error::RecvError::Closed) => break,
      Err(broadcast::error::RecvError::Lagged(_)) => {}
    }
  }
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

/// Start the web module: spawns a log consumer task and the HTTP server.
pub async fn start(rx: broadcast::Receiver<LogEvent>, port: u16) {
  let store = Arc::new(LogStore::new());

  // Spawn the log consumer that bridges core events to the web LogStore
  let consumer_store = Arc::clone(&store);
  tokio::spawn(log_consumer(rx, consumer_store));

  let app = Router::new()
    .route("/api/sse", get(sse::handler))
    .fallback(assets::handler)
    .with_state(store)
    .layer(SetResponseHeaderLayer::overriding(
      HeaderName::from_static("cross-origin-opener-policy"),
      HeaderValue::from_static("same-origin"),
    ))
    .layer(SetResponseHeaderLayer::overriding(
      HeaderName::from_static("cross-origin-embedder-policy"),
      HeaderValue::from_static("credentialless"),
    ));

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
