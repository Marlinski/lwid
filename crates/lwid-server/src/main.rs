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
use lwid_server::config::{CliArgs, Config, StorageBackend};
use lwid_server::db;
use lwid_server::reaper;
use lwid_common::kv::{FsKvStore, KvStore};
use lwid_common::project::{FsProjectStore, ProjectStore};
use lwid_common::store::{BlobStore, FsBlobStore};

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

    let (blob_store, project_store, kv_store) = build_stores(&config)?;

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
        blobs: blob_store,
        projects: project_store,
        kv: kv_store,
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
    // Legacy hostnames answer with a 301 to the canonical one. Applied
    // outermost so it runs before routing — a redirect must not depend on the
    // path existing.
    if let Some(ref canonical) = config.server.canonical_host {
        if !config.server.redirect_hosts.is_empty() {
            info!(
                "redirecting {:?} -> https://{}",
                config.server.redirect_hosts, canonical,
            );
        }
    }

    let app = app
        // Sandbox hosts (<label>.<sandbox_base_domain>) answer only the
        // bridge page and its SW. Inside the canonical-host redirect, so a
        // legacy hostname that happens to sit under the sandbox base domain
        // (www.<base>) is still redirected rather than 404'd.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            lwid_server::sandbox::gate_sandbox_hosts,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            lwid_server::redirect::canonical_host,
        ))
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

/// Construct the three storage backends according to `storage.backend`.
///
/// Defaults to the filesystem; S3 is used only when explicitly selected, and
/// then every object lives in the bucket — no persistent volume is needed
/// unless authentication is enabled (SQLite still wants a real file).
type Stores = (Arc<dyn BlobStore>, Arc<dyn ProjectStore>, Arc<dyn KvStore>);

fn build_stores(config: &Config) -> Result<Stores, Box<dyn std::error::Error>> {
    match config.storage.backend {
        StorageBackend::Fs => {
            let dir = &config.storage.data_dir;
            info!("storage backend: fs ({})", dir.display());
            Ok((
                Arc::new(FsBlobStore::new(dir.join("blobs"))?),
                Arc::new(FsProjectStore::new(dir.join("projects"))?),
                Arc::new(FsKvStore::new(dir.join("store"))?),
            ))
        }

        #[cfg(feature = "s3")]
        StorageBackend::S3 => {
            use lwid_common::s3::{client, S3BlobStore, S3KvStore, S3ProjectStore};

            let settings = config.storage.s3_settings()?;
            let prefix = settings.normalized_prefix();
            info!(
                "storage backend: s3 (endpoint={}, bucket={}, prefix={:?}, path_style={})",
                settings.endpoint, settings.bucket, prefix, settings.force_path_style,
            );

            let client = client(&settings);
            Ok((
                Arc::new(S3BlobStore::new(
                    client.clone(),
                    settings.bucket.clone(),
                    prefix.clone(),
                )),
                Arc::new(S3ProjectStore::new(
                    client.clone(),
                    settings.bucket.clone(),
                    prefix.clone(),
                )),
                Arc::new(S3KvStore::new(client, settings.bucket, prefix)),
            ))
        }

        #[cfg(not(feature = "s3"))]
        StorageBackend::S3 => Err(
            "storage.backend = \"s3\" but this binary was built without the `s3` feature".into(),
        ),
    }
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
