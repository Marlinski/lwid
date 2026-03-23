//! Global user configuration: `~/.config/lwid/config.toml`.
//!
//! Only `server` is supported for now. The file format is a minimal TOML
//! subset — we parse it without pulling in a TOML crate:
//!
//! ```toml
//! [defaults]
//! server = "https://lookwhatidid.ovh"
//! ```

use std::path::PathBuf;

use lwid_common::limits::DEFAULT_SERVER;

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("lwid").join("config.toml"))
}

/// Return the default server URL.
///
/// Priority (highest first):
///   1. `~/.config/lwid/config.toml` — written by the installer
///   2. Compile-time `LWID_DEFAULT_SERVER` env var (falls back to lookwhatidid.xyz)
pub fn default_server() -> String {
    if let Some(path) = config_path() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("server") {
                    let rest = rest.trim();
                    if let Some(rest) = rest.strip_prefix('=') {
                        let value = rest.trim().trim_matches('"');
                        if !value.is_empty() {
                            return value.to_owned();
                        }
                    }
                }
            }
        }
    }
    DEFAULT_SERVER.to_owned()
}

/// Write `server` to `~/.config/lwid/config.toml`.
///
/// Called by tests; not used by the CLI itself (the installer writes this).
#[cfg(test)]
pub fn save_default_server(server: &str) -> std::io::Result<()> {
    let path = config_path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "cannot determine config dir")
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("[defaults]\nserver = \"{server}\"\n"))
}
