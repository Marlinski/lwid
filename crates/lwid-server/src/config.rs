//! Layered configuration for LookWhatIDid.
//!
//! Resolution order (highest priority wins):
//!   CLI flags → environment variables → config file → compiled defaults

use std::path::{Path, PathBuf};

use clap::Parser;
use serde::Deserialize;

// ── Defaults ────────────────────────────────────────────────────────────────

const DEFAULT_DATA_DIR: &str = "./data";
const DEFAULT_LISTEN: &str = "0.0.0.0:8080";
const DEFAULT_MAX_BLOB_SIZE: usize = lwid_common::limits::MAX_BLOB_SIZE;
const DEFAULT_SHELL_DIR: &str = "./shell";

// ── Error type ──────────────────────────────────────────────────────────────

/// Errors that can occur while loading configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse config file: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("invalid environment variable value for {key}: {reason}")]
    EnvVar { key: &'static str, reason: String },

    #[error("missing required configuration: {0}")]
    Missing(String),
}

// ── CLI arguments ───────────────────────────────────────────────────────────

/// Encrypted backendless app-sharing platform.
#[derive(Debug, Parser)]
#[command(name = "lookwhatidid", version, about)]
pub struct CliArgs {
    /// Path to the TOML configuration file.
    #[arg(long, default_value = "config.toml")]
    pub config: PathBuf,

    /// Override `storage.data_dir`.
    #[arg(long)]
    pub data_dir: Option<PathBuf>,

    /// Override `server.listen`.
    #[arg(long)]
    pub listen: Option<String>,

    /// Override `server.max_blob_size` (bytes).
    #[arg(long)]
    pub max_blob_size: Option<usize>,

    /// Override `server.shell_dir` (path to the shell SPA directory).
    #[arg(long)]
    pub shell_dir: Option<PathBuf>,
}

// ── Configuration structs ───────────────────────────────────────────────────

/// Top-level configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub storage: StorageConfig,
    pub server: ServerConfig,
    pub policy: PolicyConfig,
    pub auth: AuthConfig,
}

/// Storage-related settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Directory where blobs and project metadata are stored.
    pub data_dir: PathBuf,
    /// Path to the SQLite database file.
    /// Defaults to `{data_dir}/lwid.db` if not set.
    pub db_path: Option<PathBuf>,
}

/// HTTP server settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Socket address to listen on (e.g. `0.0.0.0:8080`).
    pub listen: String,

    /// Maximum size of a single blob upload, in bytes.
    pub max_blob_size: usize,

    /// Allowed CORS origins. Use `["*"]` to allow everything.
    pub cors_origins: Vec<String>,

    /// Directory containing the shell SPA files (`index.html`, `js/`, `css/`, `sw.js`).
    pub shell_dir: PathBuf,

    /// Public base URL of the server (used for OAuth redirect URIs and magic links).
    /// E.g. `https://lookwhatidid.xyz`. Defaults to `http://localhost:8080`.
    pub base_url: String,
}

// ── Policy / quota tiers ────────────────────────────────────────────────────

/// Quota and TTL limits per tier.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TierPolicy {
    /// Maximum size of a single blob upload, in bytes.
    pub max_blob_size: usize,
    /// Maximum total plaintext size of a project, in bytes.
    pub max_project_size: usize,
    /// Maximum total size of the KV/blob store per project, in bytes.
    pub max_store_total: usize,
    /// Maximum TTL string allowed at project creation (e.g. "7d", "30d", "never").
    pub max_ttl: String,
    /// Maximum number of live (non-expired) projects. 0 = unlimited.
    pub max_projects: usize,
}

/// Tier policies for all three tiers.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PolicyConfig {
    pub anonymous: TierPolicy,
    pub free: TierPolicy,
    pub pro: TierPolicy,
}

// ── Auth provider configs ────────────────────────────────────────────────────

/// GitHub OAuth2 provider config. Only active when both fields are set.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GitHubAuthConfig {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

/// Google OAuth2 provider config. Only active when both fields are set.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GoogleAuthConfig {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

/// Email magic-link config. Only active when smtp_host is set.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct EmailAuthConfig {
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_user: Option<String>,
    pub smtp_password: Option<String>,
    pub from_address: Option<String>,
    /// Override the TLS SNI hostname used for cert verification.
    /// Useful when connecting via SSH tunnel (smtp_host=localhost) but the
    /// cert is issued for the real hostname (e.g. mail.lookwhatidid.xyz).
    pub smtp_tls_hostname: Option<String>,
    /// How long a magic link is valid. Defaults to "15m".
    pub magic_link_ttl_minutes: Option<u64>,
}

/// Top-level auth configuration.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AuthConfig {
    /// Secret key for signing session cookies (min 32 bytes, base64 or plain string).
    /// Required if any auth provider is enabled.
    pub session_secret: Option<String>,
    /// Session cookie TTL in days. Defaults to 30.
    pub session_ttl_days: Option<u64>,
    pub github: GitHubAuthConfig,
    pub google: GoogleAuthConfig,
    pub email: EmailAuthConfig,
}

// ── Default impls ───────────────────────────────────────────────────────────

impl Default for Config {
    fn default() -> Self {
        Self {
            storage: StorageConfig::default(),
            server: ServerConfig::default(),
            policy: PolicyConfig::default(),
            auth: AuthConfig::default(),
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from(DEFAULT_DATA_DIR),
            db_path: None,
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: DEFAULT_LISTEN.to_owned(),
            max_blob_size: DEFAULT_MAX_BLOB_SIZE,
            cors_origins: vec!["*".to_owned()],
            shell_dir: PathBuf::from(DEFAULT_SHELL_DIR),
            base_url: "http://localhost:8080".to_owned(),
        }
    }
}

impl Default for TierPolicy {
    fn default() -> Self {
        // Sensible anonymous defaults — overridden per tier below.
        Self {
            max_blob_size: 10 * 1024 * 1024,
            max_project_size: 10 * 1024 * 1024,
            max_store_total: 50 * 1024 * 1024,
            max_ttl: "7d".to_owned(),
            max_projects: 5,
        }
    }
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            anonymous: TierPolicy {
                max_blob_size: 10 * 1024 * 1024,
                max_project_size: 10 * 1024 * 1024,
                max_store_total: 50 * 1024 * 1024,
                max_ttl: "7d".to_owned(),
                max_projects: 3,
            },
            free: TierPolicy {
                max_blob_size: 10 * 1024 * 1024,
                max_project_size: 50 * 1024 * 1024,
                max_store_total: 100 * 1024 * 1024,
                max_ttl: "30d".to_owned(),
                max_projects: 20,
            },
            pro: TierPolicy {
                max_blob_size: 100 * 1024 * 1024,
                max_project_size: 500 * 1024 * 1024,
                max_store_total: 1024 * 1024 * 1024,
                max_ttl: "never".to_owned(),
                max_projects: 0, // unlimited
            },
        }
    }
}

impl StorageConfig {
    /// Resolved path to the SQLite database file.
    pub fn resolved_db_path(&self) -> PathBuf {
        self.db_path
            .clone()
            .unwrap_or_else(|| self.data_dir.join("lwid.db"))
    }
}

impl AuthConfig {
    /// Returns true if at least one auth provider is enabled.
    pub fn any_provider_enabled(&self) -> bool {
        self.github_enabled() || self.google_enabled() || self.email_enabled()
    }

    pub fn github_enabled(&self) -> bool {
        is_set(&self.github.client_id) && is_set(&self.github.client_secret)
    }

    pub fn google_enabled(&self) -> bool {
        is_set(&self.google.client_id) && is_set(&self.google.client_secret)
    }

    pub fn email_enabled(&self) -> bool {
        is_set(&self.email.smtp_host)
    }

    /// Session secret as bytes. Panics if called when no provider is enabled.
    pub fn session_secret_bytes(&self) -> Vec<u8> {
        self.session_secret
            .as_deref()
            .unwrap_or("change-me-in-production-32bytes!!")
            .as_bytes()
            .to_vec()
    }

    pub fn session_ttl_days(&self) -> u64 {
        self.session_ttl_days.unwrap_or(30)
    }
}

impl GitHubAuthConfig {
    pub fn client_id(&self) -> &str {
        self.client_id.as_deref().unwrap_or("")
    }
    pub fn client_secret(&self) -> &str {
        self.client_secret.as_deref().unwrap_or("")
    }
}

impl GoogleAuthConfig {
    pub fn client_id(&self) -> &str {
        self.client_id.as_deref().unwrap_or("")
    }
    pub fn client_secret(&self) -> &str {
        self.client_secret.as_deref().unwrap_or("")
    }
}

impl EmailAuthConfig {
    pub fn magic_link_ttl_minutes(&self) -> u64 {
        self.magic_link_ttl_minutes.unwrap_or(15)
    }
}

// ── Loading logic ───────────────────────────────────────────────────────────

impl Config {
    /// Load configuration by merging the four layers in order:
    /// 1. Compiled defaults
    /// 2. Config file (if present)
    /// 3. Environment variables (`LWID_*`)
    /// 4. CLI flags
    pub fn load(cli: &CliArgs) -> Result<Self, ConfigError> {
        // 1 + 2: defaults, optionally overlaid by the TOML file.
        let mut cfg = Self::from_file_or_default(&cli.config)?;

        // 3: environment variable overrides.
        cfg.apply_env()?;

        // 4: CLI flag overrides.
        cfg.apply_cli(cli);

        Ok(cfg)
    }

    /// Read the config file if it exists; otherwise return defaults.
    fn from_file_or_default(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(path)?;
        let cfg: Config = toml::from_str(&contents)?;
        Ok(cfg)
    }

    /// Apply `LWID_*` environment variable overrides.
    fn apply_env(&mut self) -> Result<(), ConfigError> {
        if let Ok(val) = std::env::var("LWID_STORAGE__DATA_DIR") {
            self.storage.data_dir = PathBuf::from(val);
        }
        if let Ok(val) = std::env::var("LWID_STORAGE__DB_PATH") {
            self.storage.db_path = Some(PathBuf::from(val));
        }

        if let Ok(val) = std::env::var("LWID_SERVER__LISTEN") {
            self.server.listen = val;
        }
        if let Ok(val) = std::env::var("LWID_SERVER__MAX_BLOB_SIZE") {
            self.server.max_blob_size = val.parse::<usize>().map_err(|e| ConfigError::EnvVar {
                key: "LWID_SERVER__MAX_BLOB_SIZE",
                reason: e.to_string(),
            })?;
        }
        if let Ok(val) = std::env::var("LWID_SERVER__CORS_ORIGINS") {
            self.server.cors_origins = val
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Ok(val) = std::env::var("LWID_SERVER__SHELL_DIR") {
            self.server.shell_dir = PathBuf::from(val);
        }
        if let Ok(val) = std::env::var("LWID_SERVER__BASE_URL") {
            self.server.base_url = val;
        }

        // Policy tier overrides
        apply_tier_env(&mut self.policy.anonymous, "ANONYMOUS")?;
        apply_tier_env(&mut self.policy.free, "FREE")?;
        apply_tier_env(&mut self.policy.pro, "PRO")?;

        // Auth overrides
        if let Ok(val) = std::env::var("LWID_AUTH__SESSION_SECRET") {
            self.auth.session_secret = Some(val);
        }
        if let Ok(val) = std::env::var("LWID_AUTH__SESSION_TTL_DAYS") {
            self.auth.session_ttl_days =
                Some(val.parse::<u64>().map_err(|e| ConfigError::EnvVar {
                    key: "LWID_AUTH__SESSION_TTL_DAYS",
                    reason: e.to_string(),
                })?);
        }
        if let Ok(val) = std::env::var("LWID_AUTH__GITHUB__CLIENT_ID") {
            self.auth.github.client_id = Some(val);
        }
        if let Ok(val) = std::env::var("LWID_AUTH__GITHUB__CLIENT_SECRET") {
            self.auth.github.client_secret = Some(val);
        }
        if let Ok(val) = std::env::var("LWID_AUTH__GOOGLE__CLIENT_ID") {
            self.auth.google.client_id = Some(val);
        }
        if let Ok(val) = std::env::var("LWID_AUTH__GOOGLE__CLIENT_SECRET") {
            self.auth.google.client_secret = Some(val);
        }
        if let Ok(val) = std::env::var("LWID_AUTH__EMAIL__SMTP_HOST") {
            self.auth.email.smtp_host = Some(val);
        }
        if let Ok(val) = std::env::var("LWID_AUTH__EMAIL__SMTP_PORT") {
            self.auth.email.smtp_port =
                Some(val.parse::<u16>().map_err(|e| ConfigError::EnvVar {
                    key: "LWID_AUTH__EMAIL__SMTP_PORT",
                    reason: e.to_string(),
                })?);
        }
        if let Ok(val) = std::env::var("LWID_AUTH__EMAIL__SMTP_USER") {
            self.auth.email.smtp_user = Some(val);
        }
        if let Ok(val) = std::env::var("LWID_AUTH__EMAIL__SMTP_PASSWORD") {
            self.auth.email.smtp_password = Some(val);
        }
        if let Ok(val) = std::env::var("LWID_AUTH__EMAIL__FROM_ADDRESS") {
            self.auth.email.from_address = Some(val);
        }
        if let Ok(val) = std::env::var("LWID_AUTH__EMAIL__SMTP_TLS_HOSTNAME") {
            self.auth.email.smtp_tls_hostname = Some(val);
        }

        Ok(())
    }

    /// Apply CLI flag overrides (only for flags that were explicitly provided).
    fn apply_cli(&mut self, cli: &CliArgs) {
        if let Some(ref dir) = cli.data_dir {
            self.storage.data_dir = dir.clone();
        }
        if let Some(ref listen) = cli.listen {
            self.server.listen = listen.clone();
        }
        if let Some(size) = cli.max_blob_size {
            self.server.max_blob_size = size;
        }
        if let Some(ref dir) = cli.shell_dir {
            self.server.shell_dir = dir.clone();
        }
    }
}

fn apply_tier_env(tier: &mut TierPolicy, name: &'static str) -> Result<(), ConfigError> {
    let prefix = format!("LWID_POLICY__{name}__");

    if let Ok(val) = std::env::var(format!("{prefix}MAX_BLOB_SIZE")) {
        tier.max_blob_size = val.parse::<usize>().map_err(|e| ConfigError::EnvVar {
            key: "LWID_POLICY__*__MAX_BLOB_SIZE",
            reason: e.to_string(),
        })?;
    }
    if let Ok(val) = std::env::var(format!("{prefix}MAX_PROJECT_SIZE")) {
        tier.max_project_size = val.parse::<usize>().map_err(|e| ConfigError::EnvVar {
            key: "LWID_POLICY__*__MAX_PROJECT_SIZE",
            reason: e.to_string(),
        })?;
    }
    if let Ok(val) = std::env::var(format!("{prefix}MAX_STORE_TOTAL")) {
        tier.max_store_total = val.parse::<usize>().map_err(|e| ConfigError::EnvVar {
            key: "LWID_POLICY__*__MAX_STORE_TOTAL",
            reason: e.to_string(),
        })?;
    }
    if let Ok(val) = std::env::var(format!("{prefix}MAX_TTL")) {
        tier.max_ttl = val;
    }
    if let Ok(val) = std::env::var(format!("{prefix}MAX_PROJECTS")) {
        tier.max_projects = val.parse::<usize>().map_err(|e| ConfigError::EnvVar {
            key: "LWID_POLICY__*__MAX_PROJECTS",
            reason: e.to_string(),
        })?;
    }
    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Returns true if the option contains a non-empty string.
/// Env vars set to `""` are treated the same as absent.
fn is_set(opt: &Option<String>) -> bool {
    opt.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let cfg = Config::default();
        assert_eq!(cfg.storage.data_dir, PathBuf::from("./data"));
        assert_eq!(cfg.server.listen, "0.0.0.0:8080");
        assert_eq!(cfg.server.max_blob_size, 10 * 1024 * 1024);
        assert_eq!(cfg.server.cors_origins, vec!["*"]);
        assert_eq!(cfg.policy.anonymous.max_ttl, "7d");
        assert_eq!(cfg.policy.free.max_ttl, "30d");
        assert_eq!(cfg.policy.pro.max_ttl, "never");
        assert_eq!(cfg.policy.pro.max_projects, 0);
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let cli = CliArgs {
            config: PathBuf::from("nonexistent.toml"),
            data_dir: None,
            listen: None,
            max_blob_size: None,
            shell_dir: None,
        };
        let cfg = Config::load(&cli).expect("should succeed with defaults");
        assert_eq!(cfg.server.listen, "0.0.0.0:8080");
    }

    #[test]
    fn db_path_defaults_to_data_dir() {
        let cfg = Config::default();
        assert_eq!(
            cfg.storage.resolved_db_path(),
            PathBuf::from("./data/lwid.db")
        );
    }

    #[test]
    fn auth_no_providers_enabled_by_default() {
        let cfg = Config::default();
        assert!(!cfg.auth.any_provider_enabled());
    }
}
