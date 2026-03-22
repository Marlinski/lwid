//! Auth HTTP handlers: /auth/me, /auth/logout, /auth/cli, /auth/cli/callback.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::Json;
use axum_extra::extract::cookie::PrivateCookieJar;
use serde::Deserialize;

use crate::api::AppState;
use crate::auth::session::{clear_session_cookie, OptionalUser, SESSION_COOKIE};
use crate::db;
use lwid_common::wire::UserResponse;

// ── /auth/me ──────────────────────────────────────────────────────────────────

/// `GET /auth/me` — return the current user (or anonymous) as JSON.
pub async fn me(State(state): State<AppState>, user: OptionalUser) -> Response {
    let enabled_providers = enabled_providers(&state);

    match user.0 {
        Some(u) => Json(UserResponse {
            id: u.id,
            email: u.email,
            display_name: u.display_name,
            tier: u.tier,
            enabled_providers,
        })
        .into_response(),
        None => Json(UserResponse {
            id: String::new(),
            email: None,
            display_name: None,
            tier: "anonymous".to_owned(),
            enabled_providers,
        })
        .into_response(),
    }
}

// ── /auth/logout ──────────────────────────────────────────────────────────────

/// `POST /auth/logout` — delete the current session and clear the cookie.
pub async fn logout(
    State(state): State<AppState>,
    user: OptionalUser,
    jar: PrivateCookieJar,
) -> Response {
    // If we have a session cookie token, delete it from the DB.
    if let Some(cookie) = jar.get(SESSION_COOKIE) {
        let token = cookie.value().to_owned();
        if let Err(e) = db::delete_session(&state.db, &token).await {
            tracing::warn!("delete_session error: {e}");
        }
    } else if let Some(u) = user.0 {
        // Fallback: delete all sessions for this user (e.g. Bearer token auth).
        if let Err(e) = db::delete_all_sessions(&state.db, &u.id).await {
            tracing::warn!("delete_all_sessions error: {e}");
        }
    }

    let jar = clear_session_cookie(jar);
    (jar, StatusCode::OK).into_response()
}

// ── /auth/cli ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CliLoginParams {
    pub callback: String,
}

/// `GET /auth/cli?callback=<url>` — serve a minimal HTML sign-in page.
pub async fn cli_login_page(
    State(state): State<AppState>,
    Query(params): Query<CliLoginParams>,
) -> Response {
    let callback = &params.callback;
    let base = &state.config.server.base_url;

    let mut provider_links = String::new();

    if state.config.auth.github_enabled() {
        provider_links.push_str(&format!(
            r#"<a class="btn" href="{base}/auth/github?cli_callback={callback}">Sign in with GitHub</a>"#
        ));
    }
    if state.config.auth.google_enabled() {
        provider_links.push_str(&format!(
            r#"<a class="btn" href="{base}/auth/google?cli_callback={callback}">Sign in with Google</a>"#
        ));
    }
    if state.config.auth.email_enabled() {
        provider_links.push_str(
            r#"<form method="post" action="/auth/magic">
  <input type="email" name="email" placeholder="your@email.com" required />
  <button type="submit">Send magic link</button>
</form>"#,
        );
    }

    if provider_links.is_empty() {
        provider_links.push_str("<p>No authentication providers are configured.</p>");
    }

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>Sign in — LookWhatIDid</title>
  <style>
    body {{ font-family: sans-serif; display: flex; justify-content: center; align-items: center; min-height: 100vh; margin: 0; background: #f5f5f5; }}
    .card {{ background: white; padding: 2rem; border-radius: 8px; box-shadow: 0 2px 8px rgba(0,0,0,.1); text-align: center; max-width: 400px; width: 100%; }}
    .btn {{ display: block; margin: 0.75rem 0; padding: 0.75rem 1.5rem; background: #0070f3; color: white; text-decoration: none; border-radius: 4px; font-size: 1rem; }}
    .btn:hover {{ background: #0060df; }}
    input {{ display: block; width: 100%; padding: 0.5rem; margin: 0.5rem 0; box-sizing: border-box; border: 1px solid #ccc; border-radius: 4px; }}
    button {{ padding: 0.5rem 1.5rem; background: #0070f3; color: white; border: none; border-radius: 4px; cursor: pointer; }}
  </style>
</head>
<body>
  <div class="card">
    <h1>Sign in</h1>
    <p>Signing in from the CLI. After signing in you will be redirected back.</p>
    {provider_links}
  </div>
</body>
</html>"#
    );

    Html(html).into_response()
}

// ── /auth/cli/callback ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CliCallbackParams {
    pub callback: String,
}

/// `GET /auth/cli/callback?callback=<url>` — create a CLI session token and
/// redirect to `callback?token=<cli_token>`.
pub async fn cli_callback(
    State(state): State<AppState>,
    Query(params): Query<CliCallbackParams>,
    user: OptionalUser,
) -> Response {
    let u = match user.0 {
        Some(u) => u,
        None => {
            // Not authenticated yet — redirect to the CLI login page so the
            // user can sign in first.
            let base = &state.config.server.base_url;
            let callback = &params.callback;
            return Redirect::to(&format!("{base}/auth/cli?callback={callback}")).into_response();
        }
    };

    let ttl = state.config.auth.session_ttl_days() as u32;
    let session = match db::create_session(&state.db, &u.id, "cli", ttl).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("cli create_session error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "session creation failed").into_response();
        }
    };

    let redirect_url = format!("{}?token={}", params.callback, session.token);
    Redirect::to(&redirect_url).into_response()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn enabled_providers(state: &AppState) -> Vec<String> {
    let mut providers = Vec::new();
    if state.config.auth.github_enabled() {
        providers.push("github".to_owned());
    }
    if state.config.auth.google_enabled() {
        providers.push("google".to_owned());
    }
    if state.config.auth.email_enabled() {
        providers.push("email".to_owned());
    }
    providers
}
