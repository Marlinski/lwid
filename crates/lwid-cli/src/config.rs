//! `.lwid.json` project configuration.

use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("no .lwid.json found in {0}")]
    NotFound(String),

    #[error("failed to read .lwid.json: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse .lwid.json: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("invalid key encoding: {0}")]
    InvalidKey(String),
}

/// On-disk format uses base64url strings for keys.
#[derive(Debug, Serialize, Deserialize)]
struct RawConfig {
    server: String,
    project_id: String,
    read_key: String,
    write_key: String,
}

/// Parsed project configuration with raw key bytes.
#[derive(Debug, Clone)]
pub struct ProjectConfig {
    pub server: String,
    pub project_id: String,
    pub read_key: Vec<u8>,
    pub write_key: Vec<u8>,
}

/// Load `.lwid.json` from the given directory.
pub fn load(dir: &str) -> Result<ProjectConfig, ConfigError> {
    let path = Path::new(dir).join(".lwid.json");
    if !path.exists() {
        return Err(ConfigError::NotFound(dir.to_string()));
    }
    let content = std::fs::read_to_string(&path)?;
    let raw: RawConfig = serde_json::from_str(&content)?;

    use base64::prelude::*;
    let read_key = BASE64_URL_SAFE_NO_PAD
        .decode(&raw.read_key)
        .map_err(|e| ConfigError::InvalidKey(format!("read_key: {e}")))?;
    let write_key = BASE64_URL_SAFE_NO_PAD
        .decode(&raw.write_key)
        .map_err(|e| ConfigError::InvalidKey(format!("write_key: {e}")))?;

    Ok(ProjectConfig {
        server: raw.server,
        project_id: raw.project_id,
        read_key,
        write_key,
    })
}

/// Save `.lwid.json` to the given directory.
pub fn save(dir: &str, cfg: &ProjectConfig) -> Result<(), ConfigError> {
    use base64::prelude::*;
    let raw = RawConfig {
        server: cfg.server.clone(),
        project_id: cfg.project_id.clone(),
        read_key: BASE64_URL_SAFE_NO_PAD.encode(&cfg.read_key),
        write_key: BASE64_URL_SAFE_NO_PAD.encode(&cfg.write_key),
    };
    let json = serde_json::to_string_pretty(&raw)?;
    let path = Path::new(dir).join(".lwid.json");
    std::fs::write(&path, json)?;
    Ok(())
}
