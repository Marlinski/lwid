//! Session cookie management and the `OptionalUser` extractor.
//!
//! # AppState requirements
//!
//! `AppState` must expose:
//!   ```ignore
//!   pub cookie_key: axum_extra::extract::cookie::Key,
//!   pub db: Arc<sqlx::SqlitePool>,
//!   ```
//! The `cookie_key` is derived from `config.auth.session_secret_bytes()` via
//! `cookie_key_from_secret`. The secret is zero-padded to 64 bytes if needed,
//! then passed to `Key::from`.
//!
//! `AppState` must also implement `axum::extract::FromRef<AppState> for Key`
//! so that `PrivateCookieJar` can be used as an axum extractor directly in
//! handler signatures.

use axum::extract::FromRequestParts;
use axum::http::{request::Parts, StatusCode};
use axum_extra::extract::cookie::{Cookie, Key, PrivateCookieJar, SameSite};
use time::Duration;

use crate::api::AppState;
use crate::db::{self, DbUser};

pub const SESSION_COOKIE: &str = "lwid_session";

// ── OptionalUser extractor ───────────────────────────────────────────────────

/// An axum extractor that resolves the current session to a `DbUser`, or
/// `None` for unauthenticated requests.
///
/// Resolution order:
/// 1. `lwid_session` private cookie (browser clients).
/// 2. `Authorization: Bearer <token>` header (CLI / API clients).
pub struct OptionalUser(pub Option<DbUser>);

impl FromRequestParts<AppState> for OptionalUser {
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // 1. Try private cookie.
        let jar = PrivateCookieJar::from_headers(&parts.headers, state.cookie_key.clone());
        if let Some(cookie) = jar.get(SESSION_COOKIE) {
            let token = cookie.value().to_owned();
            match db::get_session(&state.db, &token).await {
                Ok(Some(user)) => return Ok(OptionalUser(Some(user))),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("session DB lookup error: {e}");
                }
            }
        }

        // 2. Try Authorization: Bearer header.
        if let Some(auth_header) = parts.headers.get(axum::http::header::AUTHORIZATION) {
            if let Ok(value) = auth_header.to_str() {
                if let Some(token) = value.strip_prefix("Bearer ") {
                    let token = token.trim().to_owned();
                    match db::get_session(&state.db, &token).await {
                        Ok(Some(user)) => return Ok(OptionalUser(Some(user))),
                        Ok(None) => {}
                        Err(e) => {
                            tracing::warn!("bearer session DB lookup error: {e}");
                        }
                    }
                }
            }
        }

        Ok(OptionalUser(None))
    }
}

// ── Cookie helpers ───────────────────────────────────────────────────────────

/// Add (or replace) the session cookie with the given `token`.
pub fn set_session_cookie(jar: PrivateCookieJar, token: &str, ttl_days: u64) -> PrivateCookieJar {
    let max_age = Duration::days(ttl_days as i64);
    let cookie = Cookie::build((SESSION_COOKIE, token.to_owned()))
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .max_age(max_age)
        .path("/")
        .build();
    jar.add(cookie)
}

/// Remove the session cookie.
pub fn clear_session_cookie(jar: PrivateCookieJar) -> PrivateCookieJar {
    jar.remove(Cookie::build(SESSION_COOKIE).path("/").build())
}

// ── Key derivation helper ────────────────────────────────────────────────────

/// Build a `Key` from arbitrary bytes.
///
/// `Key::from` requires at least 64 bytes. The input is zero-padded as needed.
pub fn cookie_key_from_secret(secret: &[u8]) -> Key {
    if secret.len() >= 64 {
        Key::from(&secret[..64])
    } else {
        let mut padded = [0u8; 64];
        padded[..secret.len()].copy_from_slice(secret);
        Key::from(&padded)
    }
}
