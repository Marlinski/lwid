//! Push command implementation.

use std::path::Path;

use base64::prelude::*;
use ed25519_dalek::{Signer, SigningKey};
use lwid_common::cid::Cid;
use lwid_common::crypto;
use lwid_common::limits::{self, MAX_BLOB_SIZE, MAX_PROJECT_SIZE};
use lwid_common::manifest::SCHEMA_ENCRYPTED_PATHS;

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

// ── Project name ────────────────────────────────────────────────────────────

/// Longest stored project name, in characters. Must match MAX_NAME_LENGTH in
/// shell/js/manifest.js — both write the same field.
const MAX_NAME_LENGTH: usize = 80;

/// Normalise a name before storing it: drop control characters, collapse
/// whitespace, cap the length. Mirrors `sanitizeName()` in the shell so a name
/// set by the CLI and one set in the browser are stored identically.
fn sanitize_name(name: &str) -> Option<String> {
    let mut out = String::new();
    let mut last_was_space = false;
    for c in name.chars() {
        if is_name_space(c) {
            if !last_was_space && !out.is_empty() {
                out.push(' ');
            }
            last_was_space = true;
            continue;
        }
        // Every other control character is dropped outright.
        if c.is_control() || ('\u{80}'..='\u{9f}').contains(&c) {
            continue;
        }
        out.push(c);
        last_was_space = false;
    }
    let trimmed: String = out.trim().chars().take(MAX_NAME_LENGTH).collect();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// The characters [`sanitize_name`] treats as whitespace, written out rather
/// than inherited from `char::is_whitespace`.
///
/// Rust and JavaScript do not agree on the edges — U+0085 is whitespace to
/// `char::is_whitespace` but not to a JS `\s` regex, and U+FEFF is the
/// reverse — and a name set here must normalise identically to one set in the
/// browser, since both land in the same manifest field. Must match SPACE_RE in
/// shell/js/manifest.js.
fn is_name_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}' | '\u{a}' | '\u{b}' | '\u{c}' | '\u{d}' | '\u{20}'
            | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}'
            | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

/// Extract the text of the first `<title>` element, if any.
///
/// Deliberately a scan rather than a parser dependency: this reads one tag out
/// of a file the user wrote, and a wrong answer costs a default label.
fn html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let gt = lower[open..].find('>')? + open + 1;
    let close = lower[gt..].find("</title>")? + gt;
    sanitize_name(&html[gt..close])
}

/// Guess a display name for a new project from its own files.
///
/// `<title>` of the entry page, else the directory name. Runs client-side
/// because it needs plaintext — the server never sees any of this.
fn derive_name(files: &[CollectedFile], dir: &Path) -> Option<String> {
    let entry = files
        .iter()
        .find(|f| f.path == "index.html")
        .or_else(|| files.iter().find(|f| f.path.ends_with(".html")));

    if let Some(entry) = entry
        && let Ok(text) = std::str::from_utf8(&entry.content)
        && let Some(title) = html_title(text)
    {
        return Some(title);
    }

    dir.file_name()
        .and_then(|n| n.to_str())
        .and_then(sanitize_name)
}

// ── Push logic ──────────────────────────────────────────────────────────────

pub async fn run(
    dir: &str,
    server: &str,
    yes: bool,
    force: bool,
    paths: &[String],
    ttl: Option<&str>,
    name: Option<&str>,
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

    // Scan for secrets unless --force is given
    if !force {
        let scan_input: Vec<(String, Vec<u8>)> = files
            .iter()
            .map(|f| (f.path.clone(), f.content.clone()))
            .collect();
        let findings = crate::secrets::scan_files(&scan_input);
        if !findings.is_empty() {
            eprintln!("\nwarning: possible secrets detected in files to be uploaded:");
            // Group by file
            let mut by_file: std::collections::BTreeMap<&str, Vec<(usize, &str, &str)>> = Default::default();
            for f in &findings {
                by_file.entry(&f.path).or_default().push((f.line, f.description, &f.preview));
            }
            for (path, descs) in &by_file {
                eprintln!("  {}", path);
                for (line, desc, preview) in descs {
                    eprintln!("    \u{2022} line {}: {}: {}", line, desc, preview);
                }
            }
            eprintln!("\nuse -f / --force to push anyway.\n");
            return Err("secrets detected — aborting push (use -f to force)".into());
        }
    }

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

        let encrypted_path = crypto::encrypt_path(&read_key, &f.path)?;
        manifest_files.push(serde_json::json!({
            "path": encrypted_path,
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
            &read_key,
        )
        .await?
    } else {
        manifest_files
    };

    let version = SCHEMA_ENCRYPTED_PATHS;

    // Name resolution, in order: --name, then whatever the previous version was
    // called, then a guess from the content. Carrying the old name forward is
    // what stops an ordinary push from silently un-naming a project, since the
    // name lives in the manifest and a manifest is a whole snapshot.
    let resolved_name: Option<String> = match name {
        Some(explicit) => sanitize_name(explicit),
        None => {
            let inherited = match parent_cid.as_deref() {
                Some(pcid) => read_manifest_name(&client, pcid, &read_key).await,
                None => None,
            };
            inherited.or_else(|| derive_name(&files, &dir_path))
        }
    };

    let mut manifest = serde_json::json!({
        "version": version,
        "parent_cid": parent_cid,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "files": manifest_files,
    });

    if let Some(ref plain) = resolved_name {
        let encrypted = crypto::encrypt_path(&read_key, plain)?;
        manifest["name"] = serde_json::Value::String(encrypted);
        eprintln!("Name: {plain}");
    }

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

/// Read and decrypt the `name` of an existing manifest, if it has one.
///
/// Never fails a push: an unreadable name is a missing label, not an error.
async fn read_manifest_name(
    client: &Client,
    manifest_cid: &str,
    read_key: &[u8; 32],
) -> Option<String> {
    let bytes = client.get_blob(manifest_cid).await.ok()?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let encoded = manifest.get("name")?.as_str()?;
    crypto::decrypt_path(read_key, encoded)
        .ok()
        .and_then(|n| sanitize_name(&n))
}

/// Merge newly pushed files with the existing manifest.
///
/// Files in `new_files` replace any existing entry with the same path. Files
/// in the previous manifest that are not in `new_files` are preserved.
///
/// Handles both legacy (plaintext path) and schema-v1 (encrypted path) manifests.
async fn merge_with_existing(
    client: &Client,
    parent_cid: &str,
    new_files: Vec<serde_json::Value>,
    read_key: &[u8; 32],
) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
    let manifest_bytes = client.get_blob(parent_cid).await?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)?;

    // Determine if the existing manifest uses encrypted paths
    let is_legacy = manifest["version"].as_u64().unwrap_or(1) < SCHEMA_ENCRYPTED_PATHS;

    let mut merged: Vec<serde_json::Value> = Vec::new();

    // new_files already have encrypted paths (just produced by the push loop).
    // Collect the *plaintext* paths of new files for dedup lookup.
    let new_plaintext_paths: std::collections::HashSet<String> = new_files
        .iter()
        .filter_map(|f| f["path"].as_str())
        .filter_map(|enc| crypto::decrypt_path(read_key, enc).ok())
        .collect();

    // Keep existing files whose plaintext path is not being replaced.
    // Re-encrypt their paths to schema-v1 format regardless of the old format.
    if let Some(existing) = manifest["files"].as_array() {
        for entry in existing {
            if let Some(raw_path) = entry["path"].as_str() {
                let plaintext_path = if is_legacy {
                    raw_path.to_string()
                } else {
                    match crypto::decrypt_path(read_key, raw_path) {
                        Ok(p) => p,
                        Err(_) => continue, // skip unreadable entries
                    }
                };
                if !new_plaintext_paths.contains(&plaintext_path) {
                    // Re-emit with encrypted path (schema-v1)
                    let encrypted_path = crypto::encrypt_path(read_key, &plaintext_path)?;
                    let mut updated = entry.clone();
                    updated["path"] = serde_json::Value::String(encrypted_path);
                    merged.push(updated);
                }
            }
        }
    }

    // Add all new files (already have encrypted paths)
    merged.extend(new_files);
    // Sort by encrypted path string (order doesn't matter semantically, just consistent)
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
        server: server.to_string(),
        project_id: resp.project_id,
        read_key: read_key.to_vec(),
        write_key,
    };

    config::save(dir, &cfg)?;
    eprintln!("Saved .lwid.json");

    Ok(cfg)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_title_is_extracted_and_normalised() {
        assert_eq!(
            html_title("<html><head><title>  My   Console </title></head>"),
            Some("My Console".to_string()),
        );
        // Attributes on the tag, and case, must not matter.
        assert_eq!(
            html_title(r#"<TITLE lang="en">Admin</TITLE>"#),
            Some("Admin".to_string()),
        );
        assert_eq!(html_title("<html><body>no title</body></html>"), None);
        assert_eq!(html_title("<title></title>"), None);
    }

    /// The same cases the browser asserts in tests/project-names.test.mjs.
    ///
    /// A name set here and one set in the shell land in the same manifest
    /// field, so the two implementations must agree character for character.
    /// Sharing the fixture is the only thing keeping two hand-written
    /// normalisers in step — this test caught them diverging on a bare
    /// newline once already.
    #[test]
    fn sanitize_matches_the_shared_fixture() {
        const FIXTURE: &str = include_str!("../../../tests/fixtures/project-name-cases.json");
        let cases: Vec<serde_json::Value> = serde_json::from_str(FIXTURE).expect("fixture parses");
        assert!(!cases.is_empty(), "fixture must not be empty");

        for case in &cases {
            let input = case["input"].as_str().expect("input is a string");
            let expected = case["expected"].as_str().map(str::to_owned);
            assert_eq!(
                sanitize_name(input),
                expected,
                "input: {input:?}",
            );
        }
    }

    #[test]
    fn sanitize_matches_the_shell_rules() {
        assert_eq!(sanitize_name("  spaced   out  "), Some("spaced out".into()));
        assert_eq!(sanitize_name("line\nbreak"), Some("line break".into()));
        assert_eq!(sanitize_name("\u{0}evil"), Some("evil".into()));
        assert_eq!(sanitize_name("   "), None);
        assert_eq!(sanitize_name(""), None);
        assert_eq!(
            sanitize_name(&"x".repeat(200)).map(|n| n.chars().count()),
            Some(MAX_NAME_LENGTH),
        );
    }

    #[test]
    fn derive_falls_back_to_the_directory_name() {
        let files = vec![CollectedFile {
            path: "data.csv".into(),
            content: b"a,b".to_vec(),
        }];
        assert_eq!(
            derive_name(&files, Path::new("/tmp/my-project")),
            Some("my-project".to_string()),
        );
    }

    #[test]
    fn derive_prefers_the_entry_title() {
        let files = vec![CollectedFile {
            path: "index.html".into(),
            content: b"<title>Dashboard</title>".to_vec(),
        }];
        assert_eq!(
            derive_name(&files, Path::new("/tmp/my-project")),
            Some("Dashboard".to_string()),
        );
    }
}
