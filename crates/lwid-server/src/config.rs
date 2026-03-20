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
}

/// Storage-related settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Directory where blobs and project metadata are stored.
    pub data_dir: PathBuf,
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
}

// ── Default impls ───────────────────────────────────────────────────────────

impl Default for Config {
    fn default() -> Self {
        Self {
            storage: StorageConfig::default(),
            server: ServerConfig::default(),
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from(DEFAULT_DATA_DIR),
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
        }
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
    fn cli_overrides_apply() {
        let cli = CliArgs {
            config: PathBuf::from("nonexistent.toml"),
            data_dir: Some(PathBuf::from("/tmp/custom")),
            listen: Some("127.0.0.1:3000".to_owned()),
            max_blob_size: Some(42),
            shell_dir: None,
        };
        let cfg = Config::load(&cli).expect("should succeed");
        assert_eq!(cfg.storage.data_dir, PathBuf::from("/tmp/custom"));
        assert_eq!(cfg.server.listen, "127.0.0.1:3000");
        assert_eq!(cfg.server.max_blob_size, 42);
    }

    #[test]
    fn env_overrides_apply() {
        // Safety: tests using env vars are inherently non-parallel-safe, but
        // cargo test runs each test binary in its own process by default.
        unsafe {
            std::env::set_var("LWID_SERVER__LISTEN", "127.0.0.1:9999");
            std::env::set_var("LWID_SERVER__CORS_ORIGINS", "https://a.com, https://b.com");
        }

        let cli = CliArgs {
            config: PathBuf::from("nonexistent.toml"),
            data_dir: None,
            listen: None,
            max_blob_size: None,
            shell_dir: None,
        };
        let cfg = Config::load(&cli).expect("should succeed");
        assert_eq!(cfg.server.listen, "127.0.0.1:9999");
        assert_eq!(
            cfg.server.cors_origins,
            vec!["https://a.com", "https://b.com"]
        );

        // Clean up.
        unsafe {
            std::env::remove_var("LWID_SERVER__LISTEN");
            std::env::remove_var("LWID_SERVER__CORS_ORIGINS");
        }
    }

    #[test]
    fn cli_takes_precedence_over_env() {
        unsafe {
            std::env::set_var("LWID_SERVER__LISTEN", "0.0.0.0:1111");
        }

        let cli = CliArgs {
            config: PathBuf::from("nonexistent.toml"),
            data_dir: None,
            listen: Some("0.0.0.0:2222".to_owned()),
            max_blob_size: None,
            shell_dir: None,
        };
        let cfg = Config::load(&cli).expect("should succeed");
        // CLI wins over env.
        assert_eq!(cfg.server.listen, "0.0.0.0:2222");

        unsafe {
            std::env::remove_var("LWID_SERVER__LISTEN");
        }
    }
}
