//! Clone command implementation.
//!
//! `lwid clone <url> [dir]`
//!
//! Parses a lwid share URL, downloads and decrypts all files into the target
//! directory, and saves `.lwid.json` so the project can be pushed later.
//!
//! URL formats:
//!   Edit:      https://server/p/{id}#{read_key}:{write_key}
//!   View-only: https://server/p/{id}#{read_key}

use std::path::Path;

use base64::prelude::*;

use crate::client::Client;
use crate::config::{self, ProjectConfig};
use crate::pull::pull_files;

/// Parse a lwid share URL into its components.
struct ParsedUrl {
    server: String,
    project_id: String,
    read_key: Vec<u8>,
    write_key: Option<Vec<u8>>,
}

fn parse_url(url: &str) -> Result<ParsedUrl, Box<dyn std::error::Error>> {
    // Split off the fragment
    let (base, fragment) = url
        .split_once('#')
        .ok_or("invalid lwid URL: missing '#' fragment with keys")?;

    // Parse server + project_id from base: https://server/p/{id}
    let (server, path) = base
        .split_once("/p/")
        .ok_or("invalid lwid URL: expected '/p/{id}' path")?;

    if server.is_empty() {
        return Err("invalid lwid URL: missing server".into());
    }
    let project_id = path.trim_end_matches('/').to_string();
    if project_id.is_empty() {
        return Err("invalid lwid URL: missing project id".into());
    }

    // Parse keys from fragment: {read_key} or {read_key}:{write_key}
    let (read_key_b64, write_key_b64) = match fragment.split_once(':') {
        Some((r, w)) => (r, Some(w)),
        None => (fragment, None),
    };

    let read_key = BASE64_URL_SAFE_NO_PAD
        .decode(read_key_b64)
        .map_err(|e| format!("invalid read key in URL: {e}"))?;

    let write_key = write_key_b64
        .map(|w| {
            BASE64_URL_SAFE_NO_PAD
                .decode(w)
                .map_err(|e| format!("invalid write key in URL: {e}"))
        })
        .transpose()?;

    Ok(ParsedUrl {
        server: server.to_string(),
        project_id,
        read_key,
        write_key,
    })
}

pub async fn run(url: &str, dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let parsed = parse_url(url)?;

    let read_key: [u8; 32] = parsed
        .read_key
        .clone()
        .try_into()
        .map_err(|_| "read_key must be 32 bytes")?;

    // Default write_key to zeroes if view-only (pull will still work, push won't)
    let write_key = parsed.write_key.unwrap_or_else(|| vec![0u8; 32]);

    let dest = Path::new(dir);
    if !dest.exists() {
        std::fs::create_dir_all(dest)?;
        eprintln!("Created directory: {dir}");
    }

    let client = Client::new(&parsed.server);
    pull_files(&client, &parsed.project_id, &read_key, dest).await?;

    let cfg = ProjectConfig {
        server: parsed.server,
        project_id: parsed.project_id,
        read_key: read_key.to_vec(),
        write_key,
    };
    config::save(dir, &cfg)?;

    Ok(())
}
