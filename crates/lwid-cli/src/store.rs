//! KV and blob store subcommands.

use base64::engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD as BASE64_URL_SAFE_NO_PAD};
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use lwid_common::limits::DEFAULT_SERVER;

/// AES-256-GCM overhead: 12-byte nonce + 16-byte tag.
const ENCRYPTION_OVERHEAD: u64 = 28;

/// Derive the store authentication token from the read key bytes.
/// Computes SHA-256("lwid-store-auth:" + base64url_no_pad(read_key)) and returns standard base64.
/// Must match the JavaScript: `deriveStoreToken(readKeyB64url)`.
pub fn derive_store_token(read_key: &[u8]) -> String {
    let read_key_b64url = BASE64_URL_SAFE_NO_PAD.encode(read_key);
    let input = format!("lwid-store-auth:{}", read_key_b64url);
    let hash = Sha256::digest(input.as_bytes());
    BASE64_STANDARD.encode(hash)
}

/// Obfuscate a key using HMAC-SHA256(read_key, original_key) -> base64url_no_pad.
/// Must match the JavaScript `obfuscateKey()` which does HMAC-SHA256 with the raw read key bytes.
fn obfuscate_key(read_key: &[u8], key: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(read_key).expect("HMAC accepts any key size");
    mac.update(key.as_bytes());
    let result = mac.finalize().into_bytes();
    BASE64_URL_SAFE_NO_PAD.encode(result)
}

const IDX_SERVER_KEY: &str = "_idx";

async fn load_key_index(
    client: &crate::client::Client,
    project_id: &str,
    read_key: &[u8; 32],
    store_token: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let encrypted = client.get_store_value(project_id, IDX_SERVER_KEY, store_token).await?;
    match encrypted {
        None => Ok(Vec::new()),
        Some(data) => {
            let plain = lwid_common::crypto::decrypt(read_key, &data)?;
            let keys: Vec<String> = serde_json::from_slice(&plain)?;
            Ok(keys)
        }
    }
}

async fn save_key_index(
    client: &crate::client::Client,
    project_id: &str,
    read_key: &[u8; 32],
    store_token: &str,
    keys: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_vec(keys)?;
    let encrypted = lwid_common::crypto::encrypt(read_key, &json)?;
    client.put_store_value(project_id, IDX_SERVER_KEY, &encrypted, store_token).await?;
    Ok(())
}

/// Run the `kv` subcommand.
/// - `lwid kv <key>` -- GET: decrypt and print value to stdout
/// - `lwid kv <key> <value>` -- SET: encrypt and store value
pub async fn run_kv(dir: &str, key: &str, value: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = crate::config::load(dir)
        .map_err(|e| format!("No .lwid.json found -- run `lwid push` first to create a project: {e}"))?;
    let client = crate::client::Client::new(DEFAULT_SERVER);
    let read_key: [u8; 32] = cfg
        .read_key
        .try_into()
        .map_err(|_| "read_key must be 32 bytes")?;
    let store_token = derive_store_token(&read_key);
    let server_key = obfuscate_key(&read_key, key);

    match value {
        None => {
            // GET
            let encrypted = client
                .get_store_value(&cfg.project_id, &server_key, &store_token)
                .await?;
            match encrypted {
                None => {
                    eprintln!("Key not found: {}", key);
                    std::process::exit(1);
                }
                Some(data) => {
                    let plain = lwid_common::crypto::decrypt(&read_key, &data)?;
                    // Print as UTF-8 string (it's JSON-serialized by the SDK)
                    let text = String::from_utf8(plain)?;
                    println!("{}", text);
                }
            }
        }
        Some(val) => {
            // SET
            let encrypted = lwid_common::crypto::encrypt(&read_key, val.as_bytes())?;
            client
                .put_store_value(&cfg.project_id, &server_key, &encrypted, &store_token)
                .await?;
            // Update key index
            let mut idx = load_key_index(&client, &cfg.project_id, &read_key, &store_token).await?;
            if !idx.contains(&key.to_string()) {
                idx.push(key.to_string());
                save_key_index(&client, &cfg.project_id, &read_key, &store_token, &idx).await?;
            }
            eprintln!("Stored: {}", key);
        }
    }

    Ok(())
}

/// Run the `blob` subcommand.
/// - `lwid blob <key>` -- GET: decrypt and write raw bytes to stdout
/// - `lwid blob <key> <file>` -- SET: read file (or stdin if "-"), encrypt and store
/// - Also supports: `cat file | lwid blob <key>` (stdin when no file arg AND stdin is not a tty)
pub async fn run_blob(dir: &str, key: &str, file: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = crate::config::load(dir)
        .map_err(|e| format!("No .lwid.json found -- run `lwid push` first to create a project: {e}"))?;
    let client = crate::client::Client::new(DEFAULT_SERVER);
    let read_key: [u8; 32] = cfg
        .read_key
        .try_into()
        .map_err(|_| "read_key must be 32 bytes")?;
    let store_token = derive_store_token(&read_key);
    let server_key = obfuscate_key(&read_key, key);

    // Determine if this is a GET or SET:
    // - If file arg is provided -> SET (read from file, or stdin if "-")
    // - If no file arg AND stdin is a pipe (not a tty) -> SET (read from stdin)
    // - If no file arg AND stdin is a tty -> GET
    let input_data = match file {
        Some(path) => {
            if path == "-" {
                // Read from stdin
                let mut buf = Vec::new();
                std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf)?;
                Some(buf)
            } else {
                Some(std::fs::read(path)?)
            }
        }
        None => {
            // Check if stdin is a tty
            use std::io::IsTerminal;
            if !std::io::stdin().is_terminal() {
                // Stdin is piped -- read from it
                let mut buf = Vec::new();
                std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf)?;
                Some(buf)
            } else {
                None // GET mode
            }
        }
    };

    match input_data {
        None => {
            // GET -- write raw bytes to stdout
            let encrypted = client
                .get_store_value(&cfg.project_id, &server_key, &store_token)
                .await?;
            match encrypted {
                None => {
                    eprintln!("Key not found: {}", key);
                    std::process::exit(1);
                }
                Some(data) => {
                    let plain = lwid_common::crypto::decrypt(&read_key, &data)?;
                    use std::io::Write;
                    std::io::stdout().write_all(&plain)?;
                }
            }
        }
        Some(data) => {
            // SET
            let encrypted = lwid_common::crypto::encrypt(&read_key, &data)?;
            client
                .put_store_value(&cfg.project_id, &server_key, &encrypted, &store_token)
                .await?;
            // Update key index
            let mut idx = load_key_index(&client, &cfg.project_id, &read_key, &store_token).await?;
            if !idx.contains(&key.to_string()) {
                idx.push(key.to_string());
                save_key_index(&client, &cfg.project_id, &read_key, &store_token, &idx).await?;
            }
            eprintln!("Stored blob: {}", key);
        }
    }

    Ok(())
}

/// Helper: load config and derive common store params.
async fn store_context(dir: &str) -> Result<(crate::config::ProjectConfig, crate::client::Client, [u8; 32], String), Box<dyn std::error::Error>> {
    let cfg = crate::config::load(dir)
        .map_err(|e| format!("No .lwid.json found -- run `lwid push` first to create a project: {e}"))?;
    let client = crate::client::Client::new(DEFAULT_SERVER);
    let read_key: [u8; 32] = cfg
        .read_key
        .clone()
        .try_into()
        .map_err(|_| "read_key must be 32 bytes")?;
    let store_token = derive_store_token(&read_key);
    Ok((cfg, client, read_key, store_token))
}

/// Format a byte size in human-readable form.
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Truncate a string to `max` chars, appending "..." if it was longer.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

/// List all kv entries: fetch each value and print `key = value` (truncated).
pub async fn run_list_kv(dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (cfg, client, read_key, store_token) = store_context(dir).await?;
    let idx = load_key_index(&client, &cfg.project_id, &read_key, &store_token).await?;

    if idx.is_empty() {
        eprintln!("No keys stored.");
        return Ok(());
    }

    // Also fetch server listing to get sizes
    let listing = client.list_store_keys(&cfg.project_id, &store_token).await?;
    let size_map: std::collections::HashMap<String, u64> = listing
        .keys
        .into_iter()
        .map(|e| (e.key, e.size))
        .collect();

    for key in &idx {
        let server_key = obfuscate_key(&read_key, key);
        let encrypted = client
            .get_store_value(&cfg.project_id, &server_key, &store_token)
            .await?;
        match encrypted {
            Some(data) => {
                match lwid_common::crypto::decrypt(&read_key, &data) {
                    Ok(plain) => {
                        let text = String::from_utf8_lossy(&plain);
                        // Replace newlines so it stays on one line
                        let oneline = text.replace('\n', "\\n").replace('\r', "");
                        println!("{} = {}", key, truncate(&oneline, 80));
                    }
                    Err(_) => {
                        // Binary data — show size instead
                        let size = size_map
                            .get(&server_key)
                            .map(|s| s.saturating_sub(ENCRYPTION_OVERHEAD))
                            .unwrap_or(0);
                        println!("{} = <binary {}>", key, format_size(size));
                    }
                }
            }
            None => {
                println!("{} = <missing>", key);
            }
        }
    }

    Ok(())
}

/// List all blob entries: show key and decrypted size.
pub async fn run_list_blob(dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (cfg, client, read_key, store_token) = store_context(dir).await?;
    let idx = load_key_index(&client, &cfg.project_id, &read_key, &store_token).await?;

    if idx.is_empty() {
        eprintln!("No keys stored.");
        return Ok(());
    }

    // Fetch server listing to get per-key sizes
    let listing = client.list_store_keys(&cfg.project_id, &store_token).await?;
    let size_map: std::collections::HashMap<String, u64> = listing
        .keys
        .into_iter()
        .map(|e| (e.key, e.size))
        .collect();

    for key in &idx {
        let server_key = obfuscate_key(&read_key, key);
        let encrypted_size = size_map.get(&server_key).copied().unwrap_or(0);
        let plain_size = encrypted_size.saturating_sub(ENCRYPTION_OVERHEAD);
        println!("{}  ({})", key, format_size(plain_size));
    }

    Ok(())
}
