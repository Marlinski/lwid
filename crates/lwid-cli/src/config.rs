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
///
/// The `server` field is optional — old configs may have it, new ones don't.
/// When absent, [`lwid_common::limits::DEFAULT_SERVER`] is used.
#[derive(Debug, Serialize, Deserialize)]
struct RawConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    server: Option<String>,
    project_id: String,
    read_key: String,
    write_key: String,
}

/// Parsed project configuration with raw key bytes.
#[derive(Debug, Clone)]
pub struct ProjectConfig {
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
        project_id: raw.project_id,
        read_key,
        write_key,
    })
}

/// Save `.lwid.json` to the given directory.
///
/// The `server` field is intentionally omitted — the default is always used.
/// After writing, this also ensures `.lwid.json` is listed in the nearest
/// `.gitignore` so that keys are not accidentally committed to version control.
pub fn save(dir: &str, cfg: &ProjectConfig) -> Result<(), ConfigError> {
    use base64::prelude::*;
    let raw = RawConfig {
        server: None,
        project_id: cfg.project_id.clone(),
        read_key: BASE64_URL_SAFE_NO_PAD.encode(&cfg.read_key),
        write_key: BASE64_URL_SAFE_NO_PAD.encode(&cfg.write_key),
    };
    let json = serde_json::to_string_pretty(&raw)?;
    let path = Path::new(dir).join(".lwid.json");
    std::fs::write(&path, json)?;

    ensure_gitignore(dir);

    Ok(())
}

/// Make sure `.lwid.json` is listed in the `.gitignore` of the project
/// directory. If a `.gitignore` already exists, we append the entry only when
/// it is not already present. If no `.gitignore` exists but the directory
/// appears to be inside a git repository, we create one.
fn ensure_gitignore(dir: &str) {
    let dir_path = Path::new(dir);
    let gitignore = dir_path.join(".gitignore");

    // Check if .lwid.json is already covered.
    if gitignore.exists() {
        if let Ok(content) = std::fs::read_to_string(&gitignore) {
            if content.lines().any(|l| l.trim() == ".lwid.json") {
                return; // already ignored
            }
            // Append to existing .gitignore
            let sep = if content.ends_with('\n') { "" } else { "\n" };
            let _ = std::fs::write(&gitignore, format!("{content}{sep}.lwid.json\n"));
            eprintln!("Added .lwid.json to existing .gitignore");
        }
        return;
    }

    // No .gitignore yet — only create one if we're inside a git repo.
    if is_inside_git_repo(dir_path) {
        let _ = std::fs::write(&gitignore, ".lwid.json\n");
        eprintln!("Created .gitignore with .lwid.json");
    }
}

/// Walk up the directory tree looking for a `.git` directory.
fn is_inside_git_repo(dir: &Path) -> bool {
    let mut current = Some(dir);
    while let Some(d) = current {
        if d.join(".git").is_dir() {
            return true;
        }
        current = d.parent();
    }
    false
}
