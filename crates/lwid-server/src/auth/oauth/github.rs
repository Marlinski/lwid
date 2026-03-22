//! GitHub OAuth2 PKCE flow.
//!
//! # AppState requirements
//!
//! `AppState` must expose:
//!   ```ignore
//!   pub oauth_states: Arc<tokio::sync::Mutex<std::collections::HashMap<String, String>>>,
//!   ```
//! keyed by CSRF state value → PKCE verifier secret string.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::PrivateCookieJar;
use oauth2::reqwest::async_http_client;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, TokenUrl,
};
use oauth2::basic::BasicClient;
use serde::Deserialize;

use crate::api::AppState;
use crate::auth::session::set_session_cookie;
use crate::db;

// ── OAuth client builder ─────────────────────────────────────────────────────

fn build_client(state: &AppState) -> BasicClient {
    let cfg = &state.config.auth.github;
    let base = &state.config.server.base_url;

    BasicClient::new(
        ClientId::new(cfg.client_id().to_owned()),
        Some(ClientSecret::new(cfg.client_secret().to_owned())),
        AuthUrl::new("https://github.com/login/oauth/authorize".to_owned())
            .expect("static URL is valid"),
        Some(
            TokenUrl::new("https://github.com/login/oauth/access_token".to_owned())
                .expect("static URL is valid"),
        ),
    )
    .set_redirect_uri(
        RedirectUrl::new(format!("{base}/auth/github/callback"))
            .expect("base_url + path must be valid"),
    )
}

// ── Query params ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CallbackParams {
    pub code: String,
    pub state: String,
}

// ── GitHub user info ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GitHubUser {
    id: u64,
    login: String,
    email: Option<String>,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// `GET /auth/github` — initiate the GitHub OAuth2 PKCE flow.
pub async fn login(State(state): State<AppState>) -> Response {
    if !state.config.auth.github_enabled() {
        return (StatusCode::NOT_FOUND, "GitHub auth not configured").into_response();
    }

    let client = build_client(&state);
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    let (auth_url, csrf_token) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("user:email".to_owned()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    // Store verifier secret keyed by CSRF state so the callback can retrieve it.
    {
        let mut map = state.oauth_states.lock().await;
        map.insert(
            csrf_token.secret().clone(),
            pkce_verifier.secret().to_owned(),
        );
    }

    Redirect::to(auth_url.as_str()).into_response()
}

/// `GET /auth/github/callback` — exchange code for token, upsert user, set cookie.
pub async fn callback(
    State(state): State<AppState>,
    Query(params): Query<CallbackParams>,
    jar: PrivateCookieJar,
) -> Response {
    if !state.config.auth.github_enabled() {
        return (StatusCode::NOT_FOUND, "GitHub auth not configured").into_response();
    }

    // Retrieve and remove the stored PKCE verifier.
    let verifier_secret = {
        let mut map = state.oauth_states.lock().await;
        match map.remove(&params.state) {
            Some(s) => s,
            None => {
                return (StatusCode::BAD_REQUEST, "invalid or expired state parameter")
                    .into_response()
            }
        }
    };

    let client = build_client(&state);
    let pkce_verifier = PkceCodeVerifier::new(verifier_secret);

    // Exchange authorization code for an access token.
    let token_result = client
        .exchange_code(AuthorizationCode::new(params.code))
        .set_pkce_verifier(pkce_verifier)
        .request_async(async_http_client)
        .await;

    let token = match token_result {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("GitHub token exchange error: {e}");
            return (StatusCode::BAD_GATEWAY, "token exchange failed").into_response();
        }
    };

    let access_token = token.access_token().secret().to_owned();

    // Fetch GitHub user info.
    let http = reqwest::Client::new();
    let gh_user: GitHubUser = match http
        .get("https://api.github.com/user")
        .header("Authorization", format!("token {access_token}"))
        .header("User-Agent", "lwid-server/1.0")
        .send()
        .await
    {
        Ok(resp) => match resp.json::<GitHubUser>().await {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("GitHub user parse error: {e}");
                return (StatusCode::BAD_GATEWAY, "failed to parse GitHub user").into_response();
            }
        },
        Err(e) => {
            tracing::warn!("GitHub user fetch error: {e}");
            return (StatusCode::BAD_GATEWAY, "failed to fetch GitHub user").into_response();
        }
    };

    // Upsert user and create a session.
    let provider_id = gh_user.id.to_string();
    let user = match db::upsert_user(
        &state.db,
        "github",
        &provider_id,
        gh_user.email.as_deref(),
        Some(&gh_user.login),
    )
    .await
    {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("upsert_user error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    let ttl = state.config.auth.session_ttl_days() as u32;
    let session = match db::create_session(&state.db, &user.id, "web", ttl).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("create_session error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "session creation failed").into_response();
        }
    };

    let jar = set_session_cookie(jar, &session.token, state.config.auth.session_ttl_days());
    (jar, Redirect::to("/")).into_response()
}
