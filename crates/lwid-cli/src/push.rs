//! Push command implementation.

use std::path::Path;

use base64::prelude::*;
use ed25519_dalek::{Signer, SigningKey};
use lwid_common::cid::Cid;
use lwid_common::crypto;
use lwid_common::limits::{self, MAX_BLOB_SIZE, MAX_PROJECT_SIZE};

use crate::client::Client;
use crate::config::{self, ProjectConfig};

// ── File collection ─────────────────────────────────────────────────────────

/// A collected file with its relative path, content, and original size.
struct CollectedFile {
    path: String,
    content: Vec<u8>,
}

/// Walk directory and collect files, skipping ignored paths.
fn collect_all_files(dir: &Path) -> Result<Vec<CollectedFile>, std::io::Error> {
    let mut files = Vec::new();
    walk_dir(dir, dir, &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn walk_dir(
    root: &Path,
    current: &Path,
    files: &mut Vec<CollectedFile>,
) -> Result<(), std::io::Error> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip hidden files/dirs, node_modules
        if name_str.starts_with('.') || name_str == "node_modules" {
            continue;
        }

        if path.is_dir() {
            walk_dir(root, &path, files)?;
        } else {
            let relative = path.strip_prefix(root).unwrap();
            let rel_str = relative.to_string_lossy().replace('\\', "/");
            let content = std::fs::read(&path)?;
            files.push(CollectedFile {
                path: rel_str,
                content,
            });
        }
    }
    Ok(())
}

/// Collect specific files/directories relative to `dir`.
fn collect_paths(dir: &Path, paths: &[String]) -> Result<Vec<CollectedFile>, std::io::Error> {
    let mut files = Vec::new();

    for p in paths {
        let full = dir.join(p);
        if !full.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("path not found: {p}"),
            ));
        }
        if full.is_dir() {
            walk_dir(dir, &full, &mut files)?;
        } else {
            let relative = full.strip_prefix(dir).unwrap();
            let rel_str = relative.to_string_lossy().replace('\\', "/");
            let content = std::fs::read(&full)?;
            files.push(CollectedFile {
                path: rel_str,
                content,
            });
        }
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);
    Ok(files)
}

// ── Pre-flight summary ──────────────────────────────────────────────────────

/// Print a file tree summary and return the total size. Returns `false` if the
/// user should abort (over limits).
fn print_staging_summary(files: &[CollectedFile]) -> bool {
    let total_size: u64 = files.iter().map(|f| f.content.len() as u64).sum();
    let max_file = files.iter().map(|f| f.content.len()).max().unwrap_or(0);

    eprintln!("Files to push:\n");

    for f in files {
        let size = limits::human_bytes(f.content.len() as u64);
        eprintln!("  {:<50} {size:>10}", f.path);
    }

    eprintln!();
    eprintln!(
        "  {} files, {} total",
        files.len(),
        limits::human_bytes(total_size)
    );
    eprintln!();

    let mut ok = true;

    if total_size > MAX_PROJECT_SIZE as u64 {
        eprintln!(
            "error: total size ({}) exceeds project limit ({})",
            limits::human_bytes(total_size),
            limits::human_bytes(MAX_PROJECT_SIZE as u64),
        );
        ok = false;
    }

    if max_file > MAX_BLOB_SIZE {
        eprintln!(
            "error: largest file ({}) exceeds blob limit ({})",
            limits::human_bytes(max_file as u64),
            limits::human_bytes(MAX_BLOB_SIZE as u64),
        );
        ok = false;
    }

    ok
}

/// Check size limits without printing. Returns an error message if violated.
fn validate_sizes(files: &[CollectedFile]) -> Result<(), String> {
    let total_size: u64 = files.iter().map(|f| f.content.len() as u64).sum();
    if total_size > MAX_PROJECT_SIZE as u64 {
        return Err(format!(
            "total size ({}) exceeds project limit ({})",
            limits::human_bytes(total_size),
            limits::human_bytes(MAX_PROJECT_SIZE as u64),
        ));
    }

    for f in files {
        if f.content.len() > MAX_BLOB_SIZE {
            return Err(format!(
                "file '{}' ({}) exceeds blob limit ({})",
                f.path,
                limits::human_bytes(f.content.len() as u64),
                limits::human_bytes(MAX_BLOB_SIZE as u64),
            ));
        }
    }

    Ok(())
}

// ── Push logic ──────────────────────────────────────────────────────────────

pub async fn run(
    dir: &str,
    server: &str,
    yes: bool,
    paths: &[String],
    ttl: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let dir_path = std::fs::canonicalize(dir)?;

    // 1. Load or create project config
    let is_new_project = config::load(dir).is_err();

    let files = if paths.is_empty() {
        collect_all_files(&dir_path)?
    } else {
        collect_paths(&dir_path, paths)?
    };

    if files.is_empty() {
        eprintln!("No files to push.");
        return Ok(());
    }

    // 2. First push: show staging summary and ask for confirmation
    if is_new_project && !yes {
        eprintln!("No .lwid.json found — this will create a new project.\n");
        let within_limits = print_staging_summary(&files);
        if !within_limits {
            return Ok(());
        }
        eprintln!("Run again with -y to confirm and push:");
        eprintln!("  lwid push -y");
        return Ok(());
    }

    // Validate sizes (even on subsequent pushes)
    validate_sizes(&files)?;

    // 3. Load or create config
    let cfg = match config::load(dir) {
        Ok(cfg) => {
            eprintln!("Found existing project: {}", cfg.project_id);
            cfg
        }
        Err(config::ConfigError::NotFound(_)) => {
            eprintln!("Creating new project...");
            create_new_project(dir, server, ttl).await?
        }
        Err(e) => return Err(e.into()),
    };

    let client = Client::new(server);
    let read_key: [u8; 32] = cfg
        .read_key
        .clone()
        .try_into()
        .map_err(|_| "read_key must be 32 bytes")?;

    eprintln!("Pushing {} files...", files.len());

    // 4. Encrypt and upload each file
    let mut manifest_files = Vec::new();
    for f in &files {
        let encrypted = crypto::encrypt(&read_key, &f.content)?;
        let cid = Cid::from_bytes(&encrypted);

        // Check if already uploaded (dedup)
        let exists = client.blob_exists(cid.as_str()).await?;
        if !exists {
            let uploaded_cid = client.upload_blob(encrypted).await?;
            assert_eq!(uploaded_cid, cid.to_string());
        } else {
            eprintln!("  skip (exists): {}", f.path);
        }

        manifest_files.push(serde_json::json!({
            "path": f.path,
            "cid": cid.to_string(),
            "size": f.content.len(),
        }));
        eprintln!("  {} -> {cid}", f.path);
    }

    // 5. Build manifest
    let project = client.get_project(&cfg.project_id).await?;
    let parent_cid = project.root_cid;

    // If this is a selective push and there's an existing manifest, merge with it
    let manifest_files = if !paths.is_empty() && parent_cid.is_some() {
        merge_with_existing(
            &client,
            parent_cid.as_deref().unwrap(),
            manifest_files,
        )
        .await?
    } else {
        manifest_files
    };

    let version = if parent_cid.is_some() { 0 } else { 1 };

    let manifest = serde_json::json!({
        "version": version,
        "parent_cid": parent_cid,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "files": manifest_files,
    });

    // Manifest is uploaded as plaintext JSON (not encrypted).
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    let manifest_cid_str = client.upload_blob(manifest_bytes).await?;

    eprintln!("Manifest CID: {manifest_cid_str}");

    // 6. Sign and update root
    let write_key_bytes: [u8; 32] = cfg.write_key[..32]
        .try_into()
        .map_err(|_| "write_key must contain at least 32 bytes for Ed25519 seed")?;
    let signing_key = SigningKey::from_bytes(&write_key_bytes);
    let signature = signing_key.sign(manifest_cid_str.as_bytes());
    let sig_b64 = BASE64_STANDARD.encode(signature.to_bytes());

    client
        .update_root(&cfg.project_id, &manifest_cid_str, &sig_b64)
        .await?;

    // 7. Print URL
    let read_key_b64 = BASE64_URL_SAFE_NO_PAD.encode(&cfg.read_key);
    let write_key_b64 = BASE64_URL_SAFE_NO_PAD.encode(&cfg.write_key);

    eprintln!("\nPushed successfully!");
    println!(
        "{server}/p/{}#{}:{}",
        cfg.project_id, read_key_b64, write_key_b64
    );

    Ok(())
}

/// Merge newly pushed files with the existing manifest.
///
/// Files in `new_files` replace any existing entry with the same path. Files
/// in the previous manifest that are not in `new_files` are preserved.
///
/// The parent manifest is plaintext JSON — no decryption needed.
async fn merge_with_existing(
    client: &Client,
    parent_cid: &str,
    new_files: Vec<serde_json::Value>,
) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
    let manifest_bytes = client.get_blob(parent_cid).await?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)?;

    let mut merged: Vec<serde_json::Value> = Vec::new();

    // Collect new file paths for quick lookup
    let new_paths: std::collections::HashSet<&str> = new_files
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();

    // Keep existing files that aren't being replaced
    if let Some(existing) = manifest["files"].as_array() {
        for entry in existing {
            if let Some(path) = entry["path"].as_str() {
                if !new_paths.contains(path) {
                    merged.push(entry.clone());
                }
            }
        }
    }

    // Add all new files
    merged.extend(new_files);
    merged.sort_by(|a, b| {
        let pa = a["path"].as_str().unwrap_or("");
        let pb = b["path"].as_str().unwrap_or("");
        pa.cmp(pb)
    });

    Ok(merged)
}

async fn create_new_project(
    dir: &str,
    server: &str,
    ttl: Option<&str>,
) -> Result<ProjectConfig, Box<dyn std::error::Error>> {
    let client = Client::new(server);

    // Generate keys
    let read_key = crypto::generate_read_key();
    let signing_key = SigningKey::generate(&mut rand_core::OsRng);
    let pubkey_bytes = signing_key.verifying_key().to_bytes();
    let pubkey_b64 = BASE64_STANDARD.encode(pubkey_bytes);

    // Derive store token so the server registers it at creation time
    let store_token = crate::store::derive_store_token(&read_key);

    // Create project on server
    let resp = client.create_project(&pubkey_b64, ttl, Some(&store_token)).await?;
    eprintln!("Created project: {}", resp.project_id);

    // write_key = raw 32-byte Ed25519 seed.
    // The browser's importEd25519PrivateKey() handles both this format and
    // the 48-byte PKCS#8 format (which the Web Crypto API natively exports).
    let write_key = signing_key.to_bytes().to_vec();

    let cfg = ProjectConfig {
        project_id: resp.project_id,
        read_key: read_key.to_vec(),
        write_key,
    };

    config::save(dir, &cfg)?;
    eprintln!("Saved .lwid.json");

    Ok(cfg)
}
