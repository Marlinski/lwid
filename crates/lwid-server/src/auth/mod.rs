//! Authentication module for lwid-server.
//!
//! Exposes session-cookie management, OAuth2 flows (GitHub, Google), email
//! magic-link auth, and the miscellaneous auth HTTP handlers.
//!
//! # AppState additions
//!
//! The following fields must be added to `api::AppState` before this module
//! compiles:
//!
//! ```ignore
//! /// Private-cookie signing key derived from `config.auth.session_secret_bytes()`.
//! pub cookie_key: axum_extra::extract::cookie::Key,
//!
//! /// In-flight OAuth2 PKCE verifiers, keyed by CSRF state token.
//! pub oauth_states: Arc<tokio::sync::Mutex<std::collections::HashMap<String, String>>>,
//!
//! /// In-flight magic-link tokens.
//! pub magic_tokens: Arc<tokio::sync::Mutex<std::collections::HashMap<String, magic::MagicTokenEntry>>>,
//! ```

pub mod handlers;
pub mod magic;
pub mod oauth;
pub mod session;

pub use session::OptionalUser;

use axum::routing::{get, post};
use axum::Router;

use crate::api::AppState;

/// Mount all auth routes and return a `Router` with state already applied.
pub fn router(state: AppState) -> Router {
    Router::new()
        // ── GitHub OAuth ────────────────────────────────────────────────
        .route("/auth/github", get(oauth::github::login))
        .route("/auth/github/callback", get(oauth::github::callback))
        // ── Google OAuth ────────────────────────────────────────────────
        .route("/auth/google", get(oauth::google::login))
        .route("/auth/google/callback", get(oauth::google::callback))
        // ── Magic link ──────────────────────────────────────────────────
        .route("/auth/magic", post(magic::send_magic_link))
        .route("/auth/magic/verify", get(magic::verify_magic_link))
        // ── Session / user info ─────────────────────────────────────────
        .route("/auth/me", get(handlers::me))
        .route("/auth/logout", post(handlers::logout))
        // ── CLI login ───────────────────────────────────────────────────
        .route("/auth/cli", get(handlers::cli_login_page))
        .route("/auth/cli/callback", get(handlers::cli_callback))
        .with_state(state)
}
