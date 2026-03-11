use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "dist"]
struct FrontendAssets;

pub(super) async fn handler(uri: Uri) -> Response {
  let path = uri.path().trim_start_matches('/');
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
