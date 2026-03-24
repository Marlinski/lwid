//! Pull command implementation.

use std::path::Path;

use lwid_common::crypto;

use crate::client::Client;
use crate::config;

/// Pull the latest version of the project whose `.lwid.json` lives in `dir`,
/// writing all files into `dir`.
pub async fn run(dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = config::load(dir)?;
    let client = Client::new(&cfg.server);
    let read_key: [u8; 32] = cfg
        .read_key
        .clone()
        .try_into()
        .map_err(|_| "read_key must be 32 bytes")?;

    pull_files(&client, &cfg.project_id, &read_key, Path::new(dir)).await
}

/// Core download logic shared by `pull` and `clone`.
pub async fn pull_files(
    client: &Client,
    project_id: &str,
    read_key: &[u8; 32],
    dest: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Get project metadata
    let project = client.get_project(project_id).await?;
    let root_cid = project
        .root_cid
        .ok_or("project has no published version")?;

    eprintln!("Root CID: {root_cid}");

    // 2. Download manifest (plaintext JSON — no decryption needed)
    let manifest_bytes = client.get_blob(&root_cid).await?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)?;

    let is_legacy = manifest["version"].as_u64().unwrap_or(1) < lwid_common::manifest::SCHEMA_ENCRYPTED_PATHS;

    let files = manifest["files"]
        .as_array()
        .ok_or("manifest has no files array")?;

    eprintln!("Version has {} files", files.len());

    // 3. Download and decrypt each file
    for file_entry in files {
        let raw_path = file_entry["path"]
            .as_str()
            .ok_or("file entry missing path")?;
        let path = if is_legacy {
            raw_path.to_string()
        } else {
            crypto::decrypt_path(read_key, raw_path)
                .map_err(|e| format!("failed to decrypt path: {e}"))?
        };
        let cid = file_entry["cid"]
            .as_str()
            .ok_or("file entry missing cid")?;

        let encrypted_content = client.get_blob(cid).await?;
        let content = crypto::decrypt(read_key, &encrypted_content)?;

        let file_path = dest.join(&path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&file_path, &content)?;
        eprintln!("  {path} ({} bytes)", content.len());
    }

    eprintln!("\nPulled {} files successfully!", files.len());
    Ok(())
}
