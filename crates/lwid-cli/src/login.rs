//! lwid login / logout — browser-based OAuth flow for the CLI.
//!
//! `lwid login`:
//!   1. Binds a local HTTP server on a random port (127.0.0.1:0).
//!   2. Opens the browser to `{server_url}/auth/cli?callback=http://127.0.0.1:{port}/callback`.
//!   3. Waits for the browser to redirect back with `?token=<bearer>`.
//!   4. Saves the token to `~/.config/lwid/token`.
//!   5. Prints "Logged in successfully."
//!
//! `lwid logout`:
//!   1. Deletes `~/.config/lwid/token` if it exists.
//!   2. Prints "Logged out."

use std::path::PathBuf;

use anyhow::{anyhow, Context};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

// ── Token file helpers ───────────────────────────────────────────────────────

fn token_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("lwid").join("token"))
}

/// Read the saved bearer token, if any.
pub fn load_token() -> Option<String> {
    let path = token_path()?;
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Write the bearer token to `~/.config/lwid/token`.
pub fn save_token(token: &str) -> std::io::Result<()> {
    let path = token_path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "cannot determine config dir")
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, token)
}

/// Delete the saved bearer token (ignores NotFound).
pub fn delete_token() -> std::io::Result<()> {
    let path = match token_path() {
        Some(p) => p,
        None => return Ok(()),
    };
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

// ── login ────────────────────────────────────────────────────────────────────

/// Run the browser-based OAuth login flow.
pub async fn login(server_url: &str) -> anyhow::Result<()> {
    // 1. Bind on a random port.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("failed to bind local callback server")?;
    let port = listener.local_addr()?.port();

    // 2. Open the browser.
    let callback_url = format!("http://127.0.0.1:{port}/callback");
    let auth_url = format!(
        "{}/auth/cli?callback={}",
        server_url.trim_end_matches('/'),
        urlencoded(&callback_url),
    );

    eprintln!("Opening browser for authentication...");
    eprintln!("If the browser does not open, visit:\n  {auth_url}");

    open::that(&auth_url).context("failed to open browser")?;

    // 3. Accept one connection and parse the token from the GET request line.
    eprintln!("Waiting for browser callback on port {port}...");
    let (mut stream, _) = listener
        .accept()
        .await
        .context("failed to accept callback connection")?;

    let mut buf = vec![0u8; 4096];
    let n = stream
        .read(&mut buf)
        .await
        .context("failed to read callback request")?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // The first line looks like: GET /callback?token=<TOKEN> HTTP/1.1
    let token = parse_token_from_request(&request)
        .ok_or_else(|| anyhow!("no token found in callback URL — authentication may have failed"))?;

    // 4. Send a minimal HTTP 200 response so the browser shows something.
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nYou can close this tab now.";
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
    drop(stream);

    // 5. Save token and report success.
    save_token(&token).context("failed to save token")?;
    println!("Logged in successfully.");

    Ok(())
}

// ── logout ───────────────────────────────────────────────────────────────────

/// Delete the saved bearer token.
pub async fn logout() -> anyhow::Result<()> {
    delete_token().context("failed to delete token")?;
    println!("Logged out.");
    Ok(())
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Percent-encode a URL so it can be embedded in a query string.
/// Only encodes the characters that strictly need encoding inside a query value.
fn urlencoded(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

/// Extract `token` query parameter from the first line of a raw HTTP request.
///
/// Input example: `GET /callback?token=abc123 HTTP/1.1`
fn parse_token_from_request(request: &str) -> Option<String> {
    let first_line = request.lines().next()?;
    // First line: METHOD PATH HTTP/VER
    let path = first_line.split_whitespace().nth(1)?;
    // Find the query string
    let query = path.split_once('?')?.1;
    // Find token= param
    for param in query.split('&') {
        if let Some(value) = param.strip_prefix("token=") {
            let decoded = percent_decode(value);
            if !decoded.is_empty() {
                return Some(decoded);
            }
        }
    }
    None
}

/// Minimal percent-decode for the token value (handles %XX sequences).
fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes: Vec<u8> = s.bytes().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(decoded) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(decoded as char);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            out.push(' ');
            i += 1;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}
