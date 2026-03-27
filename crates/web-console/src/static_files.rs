//! Embedded static file serving for the React frontend.
//!
//! In release builds, frontend assets are embedded in the binary via `rust-embed`.
//! In debug builds, files are read from disk for hot-reload compatibility.

use axum::{
    Router,
    routing::get,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "frontend/dist/"]
struct FrontendAssets;

/// Serve a static file or fall back to index.html for SPA routing.
async fn static_handler(
    axum::extract::Path(path): axum::extract::Path<String>,
) -> Response {
    serve_file(&path)
}

/// Serve index.html for the root path.
async fn index_handler() -> Response {
    serve_file("index.html")
}

fn serve_file(path: &str) -> Response {
    match FrontendAssets::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref())],
                file.data.to_vec(),
            )
                .into_response()
        }
        None => {
            // SPA fallback: serve index.html for unmatched paths
            match FrontendAssets::get("index.html") {
                Some(index) => {
                    (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, "text/html")],
                        index.data.to_vec(),
                    )
                        .into_response()
                }
                None => (StatusCode::NOT_FOUND, "Frontend not built").into_response(),
            }
        }
    }
}

pub fn router() -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/{*path}", get(static_handler))
}
