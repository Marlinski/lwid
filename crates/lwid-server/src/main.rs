use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use clap::Parser;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;
use tracing::info;

use lwid_server::api::{self, AppState};
use lwid_server::auth;
use lwid_server::auth::session::cookie_key_from_secret;
use lwid_server::config::{CliArgs, Config};
use lwid_server::db;
use lwid_server::reaper;
use lwid_common::kv::FsKvStore;
use lwid_common::project::FsProjectStore;
use lwid_common::store::FsBlobStore;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("lwid version: {}", env!("LWID_VERSION"));

    let cli = CliArgs::parse();
    let config = Config::load(&cli)?;

    let blob_store = FsBlobStore::new(config.storage.data_dir.join("blobs"))?;
    let project_store = FsProjectStore::new(config.storage.data_dir.join("projects"))?;
    let kv_store = FsKvStore::new(config.storage.data_dir.join("store"))?;

    // Initialize the SQLite database pool — only when authentication is
    // enabled. With no auth provider configured there are no users, sessions,
    // or ownership records, so the server runs fully stateless (no relational
    // store, no persistent volume required).
    let pool = if config.auth.any_provider_enabled() {
        let db_path = config.storage.resolved_db_path();
        info!("auth enabled — initializing SQLite at {}", db_path.display());
        Some(Arc::new(db::init_pool(&db_path).await?))
    } else {
        info!("auth disabled (no provider configured) — running stateless, SQLite not initialized");
        None
    };

    // Derive the cookie signing key from the session secret (zero-padded to 64 bytes).
    let cookie_key = cookie_key_from_secret(&config.auth.session_secret_bytes());

    let state = AppState {
        blobs: Arc::new(blob_store),
        projects: Arc::new(project_store),
        kv: Arc::new(kv_store),
        config: config.clone(),
        db: pool,
        cookie_key,
        oauth_states: Arc::new(Mutex::new(HashMap::new())),
        magic_tokens: Arc::new(Mutex::new(HashMap::new())),
    };

    let cors = build_cors(&config.server.cors_origins);

    // Body limit = max_blob_size + small margin for headers/framing.
    // This prevents axum from buffering arbitrarily large bodies into memory
    // before the application-level check in the blob upload handler.
    let body_limit = config.server.max_blob_size + 4096;

    // Mount the auth routes only when a provider is enabled; otherwise they
    // would depend on a DB pool that does not exist.
    let mut app = api::router(state.clone());
    if config.auth.any_provider_enabled() {
        app = app.merge(auth::router(state.clone()));
    }
    let app = app
        .layer(cors)
        .layer(DefaultBodyLimit::max(body_limit));

    // Start background reaper for expired projects.
    reaper::spawn(state.projects.clone(), state.blobs.clone(), state.kv.clone());

    let listener = tokio::net::TcpListener::bind(&config.server.listen).await?;
    info!("listening on {}", config.server.listen);
    info!("shell dir: {}", config.server.shell_dir.display());

    axum::serve(listener, app).await?;

    Ok(())
}

/// Build CORS middleware from the configured origins list.
fn build_cors(origins: &[String]) -> CorsLayer {
    use axum::http::Method;
    use tower_http::cors::Any;

    let methods = vec![
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::HEAD,
        Method::OPTIONS,
    ];

    if origins.iter().any(|o| o == "*") {
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(methods)
            .allow_headers(Any)
    } else {
        let parsed: Vec<axum::http::HeaderValue> = origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();

        CorsLayer::new()
            .allow_origin(parsed)
            .allow_methods(methods)
            .allow_headers(Any)
    }
}
