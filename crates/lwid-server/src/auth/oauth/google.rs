//! Google OAuth2 PKCE flow.
//!
//! # AppState requirements
//!
//! Same as `oauth::github` — uses `state.oauth_states` for PKCE verifier storage.

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
    let cfg = &state.config.auth.google;
    let base = &state.config.server.base_url;

    BasicClient::new(
        ClientId::new(cfg.client_id().to_owned()),
        Some(ClientSecret::new(cfg.client_secret().to_owned())),
        AuthUrl::new("https://accounts.google.com/o/oauth2/v2/auth".to_owned())
            .expect("static URL is valid"),
        Some(
            TokenUrl::new("https://oauth2.googleapis.com/token".to_owned())
                .expect("static URL is valid"),
        ),
    )
    .set_redirect_uri(
        RedirectUrl::new(format!("{base}/auth/google/callback"))
            .expect("base_url + path must be valid"),
    )
}

// ── Query params ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CallbackParams {
    pub code: String,
    pub state: String,
}

// ── Google userinfo ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GoogleUser {
    id: String,
    email: Option<String>,
    name: Option<String>,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// `GET /auth/google` — initiate the Google OAuth2 PKCE flow.
pub async fn login(State(state): State<AppState>) -> Response {
    if !state.config.auth.google_enabled() {
        return (StatusCode::NOT_FOUND, "Google auth not configured").into_response();
    }

    let client = build_client(&state);
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    let (auth_url, csrf_token) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("openid".to_owned()))
        .add_scope(Scope::new("email".to_owned()))
        .add_scope(Scope::new("profile".to_owned()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    {
        let mut map = state.oauth_states.lock().await;
        map.insert(
            csrf_token.secret().clone(),
            pkce_verifier.secret().to_owned(),
        );
    }

    Redirect::to(auth_url.as_str()).into_response()
}

/// `GET /auth/google/callback` — exchange code for token, upsert user, set cookie.
pub async fn callback(
    State(state): State<AppState>,
    Query(params): Query<CallbackParams>,
    jar: PrivateCookieJar,
) -> Response {
    if !state.config.auth.google_enabled() {
        return (StatusCode::NOT_FOUND, "Google auth not configured").into_response();
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

    let token_result = client
        .exchange_code(AuthorizationCode::new(params.code))
        .set_pkce_verifier(pkce_verifier)
        .request_async(async_http_client)
        .await;

    let token = match token_result {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("Google token exchange error: {e}");
            return (StatusCode::BAD_GATEWAY, "token exchange failed").into_response();
        }
    };

    let access_token = token.access_token().secret().to_owned();

    // Fetch Google userinfo.
    let http = reqwest::Client::new();
    let google_user: GoogleUser = match http
        .get("https://www.googleapis.com/oauth2/v2/userinfo")
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
    {
        Ok(resp) => match resp.json::<GoogleUser>().await {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("Google userinfo parse error: {e}");
                return (StatusCode::BAD_GATEWAY, "failed to parse Google user").into_response();
            }
        },
        Err(e) => {
            tracing::warn!("Google userinfo fetch error: {e}");
            return (StatusCode::BAD_GATEWAY, "failed to fetch Google user").into_response();
        }
    };

    let user = match db::upsert_user(
        &state.db,
        "google",
        &google_user.id,
        google_user.email.as_deref(),
        google_user.name.as_deref(),
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
