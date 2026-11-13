//! Playground assets compiled into the binary.

use axum::{
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

/// Runtime playground assets, compiled into the binary so the daemon works from any
/// working directory (e.g. under launchd/systemd, where cwd is `/`).
#[derive(RustEmbed)]
#[folder = "playground/"]
#[include = "index.html"]
#[include = "src/*"]
struct PlaygroundAssets;

pub(super) fn serve_playground_asset(path: &str) -> Response {
    match PlaygroundAssets::get(path) {
        Some(file) => (
            [
                (header::CONTENT_TYPE, file.metadata.mimetype().to_string()),
                (header::CACHE_CONTROL, "no-cache".to_string()),
            ],
            file.data,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
