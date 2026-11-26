//! Cache validation for the shell's own files.
//!
//! `ServeDir` answers with `Last-Modified` and no `Cache-Control`. Browsers
//! treat that as permission to reuse their copy for a guessed while (a
//! fraction of the file's age) without asking. After an upgrade, a returning
//! visitor then runs the new `index.html` against old `/js/*.js` modules, and
//! the page breaks on a missing export.
//!
//! So every shell file goes out with `Cache-Control: no-cache` (keep a copy,
//! but ask before using it) and an `ETag` computed from its bytes. The ETag
//! is the only validator: `Last-Modified` and `If-Modified-Since` are
//! dropped, because a file's timestamp says nothing about its content: an
//! image built from an older checkout, or with normalised mtimes, would
//! otherwise answer 304 to a browser holding the previous version.
//!
//! Left alone: `/api/*` and `/auth/*` (dynamic), and any response that
//! already chose its own `Cache-Control` (immutable blobs, the no-store
//! sandbox bridge).

use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

/// Files above this are passed through without an ETag (still `no-cache`,
/// still `Last-Modified`) rather than buffered whole to hash. The shell's
/// own files are far below it.
const MAX_HASHED_BYTES: usize = 8 * 1024 * 1024;

fn is_shell_path(path: &str) -> bool {
    !(path.starts_with("/api/") || path.starts_with("/auth/"))
}

/// Strong ETag: the first 128 bits of the body's SHA-256, hex-encoded.
fn etag_for(body: &[u8]) -> String {
    let digest = Sha256::digest(body);
    let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!("\"{hex}\"")
}

/// Does an `If-None-Match` header value match `etag`? Handles `*`, lists,
/// and weak forms (`W/"…"`), which compare equal under the weak comparison
/// RFC 9110 prescribes for `If-None-Match`.
fn if_none_match_hits(header_value: &str, etag: &str) -> bool {
    header_value.split(',').map(str::trim).any(|candidate| {
        candidate == "*" || candidate.trim_start_matches("W/") == etag
    })
}

/// Middleware: `no-cache` + content ETag on the shell's own files.
pub async fn revalidate(mut request: Request, next: Next) -> Response {
    let method = request.method().clone();
    if !(method == Method::GET || method == Method::HEAD) || !is_shell_path(request.uri().path()) {
        return next.run(request).await;
    }

    // Answered below from the ETag alone; never let ServeDir decide from
    // timestamps (see the module docs).
    request.headers_mut().remove(header::IF_MODIFIED_SINCE);
    let if_none_match = request
        .headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let response = next.run(request).await;
    if response.headers().contains_key(header::CACHE_CONTROL) {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    parts
        .headers
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));

    // Only a full 200 GET has the bytes an ETag must describe. A HEAD body
    // is empty, and a 206 is a slice.
    let hashable = method == Method::GET
        && parts.status == StatusCode::OK
        && parts
            .headers
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok())
            .is_some_and(|len| len <= MAX_HASHED_BYTES);
    if !hashable {
        return Response::from_parts(parts, body);
    }

    let bytes = match to_bytes(body, MAX_HASHED_BYTES).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!(error = %e, "static_cache: failed to buffer shell file");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let etag = etag_for(&bytes);
    parts.headers.remove(header::LAST_MODIFIED);
    parts
        .headers
        .insert(header::ETAG, HeaderValue::from_str(&etag).expect("hex etag is a valid header"));

    if if_none_match.is_some_and(|v| if_none_match_hits(&v, &etag)) {
        let mut not_modified = Response::new(Body::empty());
        *not_modified.status_mut() = StatusCode::NOT_MODIFIED;
        // A 304 carries the validators and caching headers, not the
        // representation's Content-Length/Type.
        for name in [header::ETAG, header::CACHE_CONTROL, header::VARY] {
            if let Some(v) = parts.headers.get(&name) {
                not_modified.headers_mut().insert(name, v.clone());
            }
        }
        return not_modified;
    }

    Response::from_parts(parts, Body::from(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etag_is_quoted_and_content_derived() {
        let a = etag_for(b"one");
        assert!(a.starts_with('"') && a.ends_with('"') && a.len() == 34);
        assert_eq!(a, etag_for(b"one"));
        assert_ne!(a, etag_for(b"two"));
    }

    #[test]
    fn if_none_match_forms() {
        let e = "\"abc\"";
        assert!(if_none_match_hits("\"abc\"", e));
        assert!(if_none_match_hits("W/\"abc\"", e));
        assert!(if_none_match_hits("\"x\", \"abc\"", e));
        assert!(if_none_match_hits("*", e));
        assert!(!if_none_match_hits("\"abd\"", e));
    }

    #[test]
    fn api_and_auth_are_not_shell_paths() {
        assert!(!is_shell_path("/api/blobs/x"));
        assert!(!is_shell_path("/auth/me"));
        assert!(is_shell_path("/"));
        assert!(is_shell_path("/p/abc"));
        assert!(is_shell_path("/js/project.js"));
        assert!(is_shell_path("/sandbox-sw.js"));
    }
}
