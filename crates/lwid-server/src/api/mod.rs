//! HTTP API layer.
//!
//! This module defines the shared [`AppState`] and assembles the top-level
//! [`axum::Router`] from the individual endpoint modules.

pub mod blobs;
pub mod projects;

use std::sync::Arc;

use axum::routing::{get, post, put};
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};

use crate::config::Config;
use lwid_common::project::ProjectStore;
use lwid_common::store::BlobStore;

// ---------------------------------------------------------------------------
// Shared application state
// ---------------------------------------------------------------------------

/// Shared application state, injected into handlers via axum `State`.
#[derive(Clone)]
pub struct AppState {
    pub blobs: Arc<dyn BlobStore>,
    pub projects: Arc<dyn ProjectStore>,
    pub config: Config,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build the top-level API router with all routes mounted.
pub fn router(state: AppState) -> Router {
    let shell_dir = &state.config.server.shell_dir;
    let index_html = shell_dir.join("index.html");

    // Static assets served directly from the shell directory
    let static_files = ServeDir::new(shell_dir);

    // The SPA index.html (used for `/` and `/p/{id}` fallback)
    let spa_fallback = ServeFile::new(&index_html);

    Router::new()
        // ── API routes (highest priority) ──────────────────────────────
        .route("/api/blobs", post(blobs::upload_blob))
        .route(
            "/api/blobs/{cid}",
            get(blobs::get_blob).head(blobs::head_blob),
        )
        .route("/api/projects", post(projects::create_project))
        .route("/api/projects/{id}", get(projects::get_project))
        .route("/api/projects/{id}/root", put(projects::update_root))
        .with_state(state)
        // ── SPA catch-all for /p/{id} (serves index.html) ─────────────
        .nest_service("/p", spa_fallback)
        // ── Static files: /js/*, /css/*, /sw.js, etc. ─────────────────
        .fallback_service(static_files)
}
