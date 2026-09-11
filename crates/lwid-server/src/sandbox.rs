//! Per-project sandbox origin redirect.
//!
//! `GET /api/sandbox/{id}` is the one fixed endpoint the shell always asks
//! for a project's sandboxed-content bridge (see shell/index.html's
//! `ensureSandboxBridge()`) — deliberately not under `/p/`, which axum
//! treats as a full wildcard tail match and refuses to register any
//! sibling route under. When `server.sandbox_base_domain` is
//! configured, this redirects a project's sandboxed content to its own
//! subdomain — real isolation, since a malicious project's script can
//! always reach anything same-origin with it (that's what the sandbox
//! iframe's `allow-same-origin` means). Without it configured, sandbox.html
//! is just served directly, same-origin, matching lwid's original
//! behavior — for deployments without that wildcard DNS/cert set up.
//!
//! sandbox.html itself lives under `/__lwid_sandbox__/` (not at the origin
//! root) so that it falls *inside* the scope its own Service Worker
//! registers with — a page outside a SW's scope never becomes that SW's
//! controlled client, and sandbox.html relies on `navigator.serviceWorker.
//! controller` being non-null on itself (see shell/__lwid_sandbox__/
//! sandbox.html's own comments for why the scope has to be narrower than
//! the whole origin in the first place).
//!
//! Either way, `GET /p/{id}#key` itself is untouched — only this one
//! sandbox-bootstrap request is affected, so old links keep working exactly
//! as before regardless of whether this feature is on.

use axum::extract::{Path, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::api::AppState;
use crate::redirect::host_without_port;

const BASE32_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";

/// Lowercase, unpadded base32 (RFC 4648). A bijection, not a hash — every
/// distinct ID still gets its own distinct label. This MUST exactly match
/// `subdomainLabel()` in shell/index.html: a project ID (nanoid, mixed-case
/// + `_`/`-`) isn't safe as a DNS label directly, since hostnames are
/// case-insensitive (two IDs differing only in case would collide onto the
/// same origin — the opposite of what this is for) and `_`/`-` aren't valid
/// at a label's edges either.
pub fn base32_encode(bytes: &[u8]) -> String {
    let mut bits: u32 = 0;
    let mut value: u32 = 0;
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    for &byte in bytes {
        value = (value << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            let idx = (value >> (bits - 5)) & 0x1f;
            out.push(BASE32_ALPHABET[idx as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        let idx = (value << (5 - bits)) & 0x1f;
        out.push(BASE32_ALPHABET[idx as usize] as char);
    }
    out
}

/// Always a redirect — to the project's own subdomain when configured, or
/// to the same-origin `/__lwid_sandbox__/sandbox.html` when not. Nothing
/// about the shell's identity travels in this URL: sandbox.html learns its
/// shell origin from the server when it's served (see [`get_bridge`]),
/// never from anything a third party could put in a query string.
pub async fn get_sandbox(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let base_url = &state.config.server.base_url;
    let target = match &state.config.server.sandbox_base_domain {
        Some(domain) => {
            let label = base32_encode(id.as_bytes());
            // Match base_url's own scheme rather than hardcoding https — a
            // plain-HTTP local/test deployment needs a plain-HTTP redirect
            // target too, not one pointing at a TLS listener that isn't there.
            let scheme = if base_url.starts_with("http://") { "http" } else { "https" };
            format!("{scheme}://{label}.{domain}{BRIDGE_PATH}")
        }
        None => BRIDGE_PATH.to_owned(),
    };
    (StatusCode::FOUND, [(header::LOCATION, target)]).into_response()
}

/// Where the bridge page lives on every sandbox origin (and on the shell's
/// own origin in fallback mode). Under `/__lwid_sandbox__/` so it falls
/// inside the narrow scope its own Service Worker registers with.
pub const BRIDGE_PATH: &str = "/__lwid_sandbox__/sandbox.html";

/// Token in shell/__lwid_sandbox__/sandbox.html that [`get_bridge`] replaces
/// with the shell's origin. Left unreplaced (the file served raw, somehow),
/// the bridge compares every incoming message's origin against this literal
/// and so accepts none — failing closed rather than open.
const SHELL_ORIGIN_PLACEHOLDER: &str = "__LWID_SHELL_ORIGIN__";

/// `scheme://host[:port]` of a URL, dropping any path/query. `base_url` is
/// operator config, so this stays deliberately simple.
fn origin_of(url: &str) -> &str {
    let url = url.trim();
    let after_scheme = url.find("://").map(|i| i + 3).unwrap_or(0);
    match url[after_scheme..].find('/') {
        Some(i) => &url[..after_scheme + i],
        None => url,
    }
}

/// Serve the bridge page with the shell's origin baked in and a
/// `frame-ancestors` policy that only lets the shell embed it.
///
/// This is what makes the bridge's "is this message from my shell?" check
/// trustworthy. The bridge is the one page on a sandbox origin that accepts
/// `LWID_SANDBOX_SET_FILES` — i.e. "run this content on this origin" — so
/// whoever it believes its shell is gets to run code on the project's
/// origin. That identity therefore has to come from the server (this
/// handler) and never from the URL: a query parameter would let any site
/// frame the bridge, name itself as the shell, and hand it arbitrary
/// content to run. `frame-ancestors` makes the same guarantee browser-
/// enforced, independently of any check the bridge's own script does.
///
/// Fallback (no `sandbox_base_domain`) mode bakes in an empty origin — the
/// bridge is same-origin with the shell by construction there (the
/// redirect is relative), so it uses its own `location.origin`. That also
/// keeps the shell working however it's reached (`127.0.0.1` vs
/// `localhost`, an alternate hostname) rather than tying it to `base_url`.
pub async fn get_bridge(State(state): State<AppState>) -> Response {
    let path = state
        .config
        .server
        .shell_dir
        .join(BRIDGE_PATH.trim_start_matches('/'));
    let Ok(html) = tokio::fs::read_to_string(&path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let (shell_origin, frame_ancestors) = match &state.config.server.sandbox_base_domain {
        Some(_) => {
            let origin = origin_of(&state.config.server.base_url).to_owned();
            (origin.clone(), origin)
        }
        None => (String::new(), "'self'".to_owned()),
    };

    let html = html.replace(SHELL_ORIGIN_PLACEHOLDER, &shell_origin);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8".to_owned()),
            (header::CONTENT_SECURITY_POLICY, format!("frame-ancestors {frame_ancestors}")),
            // Contents depend on live config, not just the file on disk.
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
        html,
    )
        .into_response()
}

// ── Host gating ─────────────────────────────────────────────────────────────

/// The only paths a sandbox host answers. Everything else a sandbox origin
/// needs — viewers, the SDK — its Service Worker fetches from the *shell's*
/// origin over CORS, so nothing else has any business being served there.
/// Without this, the Host-agnostic static fallback would hand out the whole
/// shell (index.html, /js, /css) and the API on every project's origin —
/// not a leak, but a working copy of the product living on an origin whose
/// entire purpose is to hold nothing a project's script could get at.
const SANDBOX_HOST_PATHS: &[&str] = &[BRIDGE_PATH, "/sandbox-sw.js"];

/// `host[:port]` of a URL.
fn host_of(url: &str) -> &str {
    let origin = origin_of(url);
    origin.find("://").map_or(origin, |i| &origin[i + 3..])
}

/// True when `host_header` names a per-project sandbox origin: a strict
/// subdomain of `base_domain` (ports ignored on both sides, matching
/// case-insensitively) that is not one of the hosts the shell itself lives
/// on. The latter exemption matters when the shell is *itself* under the
/// sandbox base domain (`app.example.com` with base `example.com`).
pub fn is_sandbox_host(host_header: &str, base_domain: &str, shell_hosts: &[&str]) -> bool {
    let host = host_without_port(host_header);
    let base = host_without_port(base_domain);
    if !host.is_ascii() || !base.is_ascii() || base.is_empty() {
        return false;
    }
    if shell_hosts
        .iter()
        .any(|h| host_without_port(h).eq_ignore_ascii_case(host))
    {
        return false;
    }
    // Need at least one label character plus the separating dot.
    if host.len() < base.len() + 2 {
        return false;
    }
    let (labels, tail) = host.split_at(host.len() - base.len());
    labels.ends_with('.') && tail.eq_ignore_ascii_case(base)
}

/// Middleware: on a sandbox host, 404 everything but the bridge page and
/// its Service Worker. A no-op when no `sandbox_base_domain` is configured
/// (fallback mode has no sandbox hosts to gate).
pub async fn gate_sandbox_hosts(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if let Some(base) = &state.config.server.sandbox_base_domain {
        let host = request
            .headers()
            .get(header::HOST)
            .and_then(|v| v.to_str().ok());
        if let Some(host) = host {
            let shell_hosts: Vec<&str> = [
                Some(host_of(&state.config.server.base_url)),
                state.config.server.canonical_host.as_deref(),
            ]
            .into_iter()
            .flatten()
            .collect();
            if is_sandbox_host(host, base, &shell_hosts)
                && !SANDBOX_HOST_PATHS.contains(&request.uri().path())
            {
                return StatusCode::NOT_FOUND.into_response();
            }
        }
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_host_detection() {
        let shell = ["lookwhatidid.xyz"];
        assert!(is_sandbox_host("mzxw6.lookwhatidid.xyz", "lookwhatidid.xyz", &shell));
        assert!(is_sandbox_host("MZXW6.LookWhatIDid.xyz", "lookwhatidid.xyz", &shell));
        // Ports on either side are ignored.
        assert!(is_sandbox_host("mzxw6.localhost:8899", "localhost:8899", &["localhost:8899"]));
        assert!(is_sandbox_host("mzxw6.localhost", "localhost:8899", &["localhost:8899"]));
        // The shell's own host is never a sandbox host, even under the base.
        assert!(!is_sandbox_host("lookwhatidid.xyz", "lookwhatidid.xyz", &shell));
        assert!(!is_sandbox_host("app.example.com", "example.com", &["app.example.com"]));
        // Lookalikes that merely end in the same characters are not subdomains.
        assert!(!is_sandbox_host("evillookwhatidid.xyz", "lookwhatidid.xyz", &shell));
        assert!(!is_sandbox_host(".lookwhatidid.xyz", "lookwhatidid.xyz", &shell));
        // Unrelated hosts.
        assert!(!is_sandbox_host("lookwhatidid.ovh", "lookwhatidid.xyz", &shell));
        assert!(!is_sandbox_host("127.0.0.1:8899", "localhost:8899", &["localhost:8899"]));
        // Degenerate config never gates anything.
        assert!(!is_sandbox_host("anything", "", &shell));
    }

    #[test]
    fn host_of_url() {
        assert_eq!(host_of("https://lookwhatidid.xyz/"), "lookwhatidid.xyz");
        assert_eq!(host_of("http://localhost:8899"), "localhost:8899");
    }

    #[test]
    fn base32_matches_known_vectors() {
        // RFC 4648 test vectors, lowercased (the RFC's own examples are
        // uppercase; this encoder is deliberately lowercase throughout).
        assert_eq!(base32_encode(b""), "");
        assert_eq!(base32_encode(b"f"), "my");
        assert_eq!(base32_encode(b"fo"), "mzxq");
        assert_eq!(base32_encode(b"foo"), "mzxw6");
        assert_eq!(base32_encode(b"foob"), "mzxw6yq");
        assert_eq!(base32_encode(b"fooba"), "mzxw6ytb");
        assert_eq!(base32_encode(b"foobar"), "mzxw6ytboi");
    }

    #[test]
    fn different_case_ids_never_collide() {
        // The whole point: two IDs differing only in case must still
        // produce two different labels (unlike naively lowercasing the ID).
        assert_ne!(base32_encode(b"AbC123"), base32_encode(b"abc123"));
    }

    #[test]
    fn output_alphabet_is_dns_safe() {
        let label = base32_encode(b"some-nanoid_ID99");
        assert!(label.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }

    #[test]
    fn origin_of_strips_path_and_keeps_port() {
        assert_eq!(origin_of("https://lookwhatidid.xyz"), "https://lookwhatidid.xyz");
        assert_eq!(origin_of("https://lookwhatidid.xyz/"), "https://lookwhatidid.xyz");
        assert_eq!(origin_of("http://localhost:8899/some/path?q=1"), "http://localhost:8899");
        assert_eq!(origin_of("  http://localhost:8080 \n"), "http://localhost:8080");
    }
}
