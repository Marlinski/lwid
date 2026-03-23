//! HTTP API layer.
//!
//! This module defines the shared [`AppState`] and assembles the top-level
//! [`axum::Router`] from the individual endpoint modules.

pub mod blobs;
pub mod projects;
pub mod skill;
pub mod store;

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{FromRef, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use axum_extra::extract::cookie::Key;
use tokio::sync::Mutex;
use tower_http::services::{ServeDir, ServeFile};

use crate::auth::magic::MagicTokenEntry;
use crate::auth::OptionalUser;
use crate::config::{Config, TierPolicy};
use lwid_common::kv::KvStore;
use lwid_common::limits::TTL_CHOICES;
use lwid_common::project::ProjectStore;
use lwid_common::store::BlobStore;
use lwid_common::wire::{ManifestAuth, ManifestPolicy, ManifestResponse, ManifestTierPolicy, VersionResponse};

// ---------------------------------------------------------------------------
// Shared application state
// ---------------------------------------------------------------------------

/// Shared application state, injected into handlers via axum `State`.
#[derive(Clone)]
pub struct AppState {
    pub blobs: Arc<dyn BlobStore>,
    pub projects: Arc<dyn ProjectStore>,
    pub kv: Arc<dyn KvStore>,
    pub config: Config,
    /// SQLite connection pool (users, sessions, project ownership).
    pub db: Arc<sqlx::SqlitePool>,
    /// Private-cookie signing key derived from `config.auth.session_secret_bytes()`.
    pub cookie_key: Key,
    /// In-flight OAuth2 PKCE verifiers, keyed by CSRF state token.
    pub oauth_states: Arc<Mutex<HashMap<String, String>>>,
    /// In-flight magic-link tokens.
    pub magic_tokens: Arc<Mutex<HashMap<String, MagicTokenEntry>>>,
}

// `PrivateCookieJar` (and `SignedCookieJar`) require `Key: FromRef<S>`.
impl FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Key {
        state.cookie_key.clone()
    }
}

// ---------------------------------------------------------------------------
// Quota helpers
// ---------------------------------------------------------------------------

/// Returns the [`TierPolicy`] for the given optional user.
pub fn tier_policy<'a>(cfg: &'a Config, user: &OptionalUser) -> &'a TierPolicy {
    match user.0.as_ref() {
        None => &cfg.policy.anonymous,
        Some(u) if u.tier == "pro" => &cfg.policy.pro,
        Some(_) => &cfg.policy.free,
    }
}

/// Clamp `requested_ttl` to `max_ttl` using the TTL ordering defined in
/// [`TTL_CHOICES`] (`["1h","1d","7d","30d","never"]`).
///
/// If the requested TTL index is higher (longer) than the max_ttl index,
/// returns `max_ttl`.  Otherwise returns `requested_ttl` unchanged.
pub fn clamp_ttl<'a>(requested_ttl: &'a str, max_ttl: &'a str) -> &'a str {
    let req_idx = TTL_CHOICES.iter().position(|&c| c == requested_ttl);
    let max_idx = TTL_CHOICES.iter().position(|&c| c == max_ttl);
    match (req_idx, max_idx) {
        (Some(ri), Some(mi)) if ri > mi => max_ttl,
        _ => requested_ttl,
    }
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
        .route("/api/version", get(get_version))
        .route("/api/manifest", get(get_manifest))
        .route("/api/blobs", post(blobs::upload_blob))
        .route(
            "/api/blobs/{cid}",
            get(blobs::get_blob).head(blobs::head_blob),
        )
        .route("/api/projects", post(projects::create_project))
        .route("/api/projects/{id}", get(projects::get_project).delete(projects::delete_project))
        .route("/api/projects/{id}/root", put(projects::update_root))
        .route("/api/projects/{id}/ttl", put(projects::extend_ttl))
        .route("/api/projects/{id}/store", get(store::list_keys))
        .route(
            "/api/projects/{id}/store/{*key}",
            get(store::get_value)
                .put(store::put_value)
                .delete(store::delete_value),
        )
        // ── Skill files: domain-aware, correct charset ─────────────────
        .route("/SKILL.md", get(skill::get_skill))
        .with_state(state)
        // ── SPA catch-all for /p/{id} (serves index.html) ─────────────
        .nest_service("/p", spa_fallback)
        // ── Static files: /js/*, /css/*, /sw.js, etc. ─────────────────
        .fallback_service(static_files)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn get_version() -> Json<VersionResponse> {
    Json(VersionResponse {
        version: env!("LWID_VERSION").to_string(),
    })
}

async fn get_manifest(State(state): State<AppState>) -> Json<ManifestResponse> {
    let cfg = &state.config;

    let mut providers = Vec::new();
    if cfg.auth.github_enabled() { providers.push("github".to_owned()); }
    if cfg.auth.google_enabled() { providers.push("google".to_owned()); }
    if cfg.auth.email_enabled()  { providers.push("email".to_owned()); }

    let enabled = !providers.is_empty();

    let tier_to_wire = |t: &crate::config::TierPolicy| ManifestTierPolicy {
        max_blob_size:    t.max_blob_size,
        max_project_size: t.max_project_size,
        max_store_total:  t.max_store_total,
        max_ttl:          t.max_ttl.clone(),
        max_projects:     t.max_projects,
    };

    Json(ManifestResponse {
        auth: ManifestAuth { enabled, providers },
        policy: ManifestPolicy {
            anonymous: tier_to_wire(&cfg.policy.anonymous),
            free:      tier_to_wire(&cfg.policy.free),
            pro:       tier_to_wire(&cfg.policy.pro),
        },
    })
}
