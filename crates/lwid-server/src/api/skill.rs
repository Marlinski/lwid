//! Handlers for /SKILL.md and /SKILL-store.md.
//!
//! Reads the Markdown file from the shell directory, replaces the
//! `{{SERVER_URL}}` placeholder with `https://<Host>` derived from the
//! incoming request, and serves the result as `text/markdown; charset=utf-8`.

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use super::AppState;

const PLACEHOLDER: &str = "{{SERVER_URL}}";

/// Serve `/SKILL.md` with `{{SERVER_URL}}` replaced by the request origin.
pub async fn get_skill(headers: HeaderMap, State(state): State<AppState>) -> Response {
    serve_skill_file("SKILL.md", &headers, &state).await
}

/// Serve `/SKILL-store.md` with `{{SERVER_URL}}` replaced by the request origin.
pub async fn get_skill_store(headers: HeaderMap, State(state): State<AppState>) -> Response {
    serve_skill_file("SKILL-store.md", &headers, &state).await
}

async fn serve_skill_file(filename: &str, headers: &HeaderMap, state: &AppState) -> Response {
    let path = state.config.server.shell_dir.join(filename);

    let content = match tokio::fs::read_to_string(&path).await {
        Ok(s) => s,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };

    // Derive the server origin from the Host header.
    // Fall back to an empty string so the placeholder is simply removed rather
    // than leaving a broken URL in the output.
    let origin = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|host| format!("https://{host}"))
        .unwrap_or_default();

    let patched = content.replace(PLACEHOLDER, &origin);

    let mut response = (StatusCode::OK, patched).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/markdown; charset=utf-8"),
    );
    response
}
