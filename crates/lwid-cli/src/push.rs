//! Push command implementation.

use std::path::Path;

use base64::prelude::*;
use ed25519_dalek::{Signer, SigningKey};
use lwid_common::cid::Cid;
use lwid_common::crypto;

use crate::client::Client;
use crate::config::{self, ProjectConfig};

/// Walk directory and collect files, skipping ignored paths.
fn collect_files(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, std::io::Error> {
    let mut files = Vec::new();
    walk_dir(dir, dir, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

fn walk_dir(
    root: &Path,
    current: &Path,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), std::io::Error> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip hidden files/dirs, .lwid.json, node_modules, .git
        if name_str.starts_with('.') || name_str == "node_modules" {
            continue;
        }

        if path.is_dir() {
            walk_dir(root, &path, files)?;
        } else {
            let relative = path.strip_prefix(root).unwrap();
            let rel_str = relative.to_string_lossy().replace('\\', "/");
            let content = std::fs::read(&path)?;
            files.push((rel_str, content));
        }
    }
    Ok(())
}

pub async fn run(dir: &str, server: &str) -> Result<(), Box<dyn std::error::Error>> {
    let dir_path = Path::new(dir);

    // 1. Load or create project config
    let cfg = match config::load(dir) {
        Ok(cfg) => {
            eprintln!("Found existing project: {}", cfg.project_id);
            cfg
        }
        Err(config::ConfigError::NotFound(_)) => {
            eprintln!("No .lwid.json found. Creating new project...");
            create_new_project(dir, server).await?
        }
        Err(e) => return Err(e.into()),
    };

    let client = Client::new(&cfg.server);
    let read_key: [u8; 32] = cfg
        .read_key
        .clone()
        .try_into()
        .map_err(|_| "read_key must be 32 bytes")?;

    // 2. Collect local files
    let files = collect_files(dir_path)?;
    if files.is_empty() {
        eprintln!("No files to push.");
        return Ok(());
    }
    eprintln!("Collected {} files", files.len());

    // 3. Encrypt and upload each file
    let mut manifest_files = Vec::new();
    for (path, content) in &files {
        let encrypted = crypto::encrypt(&read_key, content)?;
        let cid = Cid::from_bytes(&encrypted);

        // Check if already uploaded (dedup)
        let exists = client.blob_exists(cid.as_str()).await?;
        if !exists {
            let uploaded_cid = client.upload_blob(encrypted).await?;
            assert_eq!(uploaded_cid, cid.to_string());
        } else {
            eprintln!("  skip (exists): {path}");
        }

        manifest_files.push(serde_json::json!({
            "path": path,
            "cid": cid.to_string(),
            "size": content.len(),
        }));
        eprintln!("  {path} → {cid}");
    }

    // 4. Build manifest
    let project = client.get_project(&cfg.project_id).await?;
    let parent_cid = project.root_cid;
    let version = if parent_cid.is_some() {
        // We'd need to fetch + decrypt the parent manifest to get its version,
        // but for simplicity just use timestamp-based versioning.
        // The actual version number doesn't matter for correctness.
        0 // Will be determined by the chain
    } else {
        1
    };

    let manifest = serde_json::json!({
        "version": version,
        "parent_cid": parent_cid,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "files": manifest_files,
    });

    let manifest_bytes = serde_json::to_vec(&manifest)?;
    let encrypted_manifest = crypto::encrypt(&read_key, &manifest_bytes)?;
    let manifest_cid_str = client.upload_blob(encrypted_manifest).await?;

    eprintln!("Manifest CID: {manifest_cid_str}");

    // 5. Sign and update root
    let write_key_bytes: [u8; 32] = cfg.write_key[..32]
        .try_into()
        .map_err(|_| "write_key must contain at least 32 bytes for Ed25519 seed")?;
    let signing_key = SigningKey::from_bytes(&write_key_bytes);
    let signature = signing_key.sign(manifest_cid_str.as_bytes());
    let sig_b64 = BASE64_STANDARD.encode(signature.to_bytes());

    client
        .update_root(&cfg.project_id, &manifest_cid_str, &sig_b64)
        .await?;

    // 6. Print URL
    let read_key_b64 = BASE64_URL_SAFE_NO_PAD.encode(&cfg.read_key);
    let write_key_b64 = BASE64_URL_SAFE_NO_PAD.encode(&cfg.write_key);

    eprintln!("\nPushed successfully!");
    println!(
        "{}/p/{}#{}:{}",
        cfg.server, cfg.project_id, read_key_b64, write_key_b64
    );

    Ok(())
}

async fn create_new_project(
    dir: &str,
    server: &str,
) -> Result<ProjectConfig, Box<dyn std::error::Error>> {
    let client = Client::new(server);

    // Generate keys
    let read_key = crypto::generate_read_key();
    let signing_key = SigningKey::generate(&mut rand_core::OsRng);
    let pubkey_bytes = signing_key.verifying_key().to_bytes();
    let pubkey_b64 = BASE64_STANDARD.encode(pubkey_bytes);

    // Create project on server
    let resp = client.create_project(&pubkey_b64).await?;
    eprintln!("Created project: {}", resp.project_id);

    // write_key = seed bytes (32 bytes of the signing key)
    let write_key = signing_key.to_bytes().to_vec();

    let cfg = ProjectConfig {
        server: server.to_string(),
        project_id: resp.project_id,
        read_key: read_key.to_vec(),
        write_key,
    };

    config::save(dir, &cfg)?;
    eprintln!("Saved .lwid.json");

    Ok(cfg)
}
