//! Email magic-link authentication.
//!
//! # AppState requirements
//!
//! `AppState` must expose:
//!   ```ignore
//!   pub magic_tokens: Arc<tokio::sync::Mutex<std::collections::HashMap<String, MagicTokenEntry>>>,
//!   ```

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use axum_extra::extract::cookie::PrivateCookieJar;
use chrono::Utc;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::Deserialize;

use crate::api::AppState;
use crate::auth::session::set_session_cookie;
use crate::db;
use lwid_common::wire::MagicLinkRequest;

// ── In-memory token store ─────────────────────────────────────────────────────

/// An entry in the in-memory magic-link token map.
#[derive(Debug, Clone)]
pub struct MagicTokenEntry {
    pub user_email: String,
    pub expires_at: chrono::DateTime<Utc>,
}

// ── Query params ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct VerifyParams {
    pub token: String,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// `POST /auth/magic` — send a magic link to the given email address.
pub async fn send_magic_link(
    State(state): State<AppState>,
    Json(req): Json<MagicLinkRequest>,
) -> Response {
    if !state.config.auth.email_enabled() {
        return (StatusCode::NOT_FOUND, "email auth not configured").into_response();
    }

    let email = req.email.trim().to_lowercase();
    let ttl_minutes = state.config.auth.email.magic_link_ttl_minutes();
    let expires_at = Utc::now()
        + chrono::Duration::minutes(ttl_minutes as i64);

    let token = nanoid::nanoid!(32);

    {
        let mut map = state.magic_tokens.lock().await;
        map.insert(
            token.clone(),
            MagicTokenEntry {
                user_email: email.clone(),
                expires_at,
            },
        );
    }

    // Attempt to send the email — failure is logged but never surfaced to
    // the caller to avoid leaking whether the address exists.
    let base_url = &state.config.server.base_url;
    let magic_url = format!("{base_url}/auth/magic/verify?token={token}");

    if let Err(e) = send_email(&state, &email, &magic_url).await {
        tracing::warn!("magic link send error for {email}: {e}");
    }

    Json(serde_json::json!({ "sent": true })).into_response()
}

/// `GET /auth/magic/verify?token=…` — verify token, create session, set cookie.
pub async fn verify_magic_link(
    State(state): State<AppState>,
    Query(params): Query<VerifyParams>,
    jar: PrivateCookieJar,
) -> Response {
    if !state.config.auth.email_enabled() {
        return (StatusCode::NOT_FOUND, "email auth not configured").into_response();
    }

    let entry = {
        let mut map = state.magic_tokens.lock().await;
        map.remove(&params.token)
    };

    let entry = match entry {
        Some(e) => e,
        None => return (StatusCode::BAD_REQUEST, "invalid or expired token").into_response(),
    };

    if Utc::now() > entry.expires_at {
        return (StatusCode::BAD_REQUEST, "token has expired").into_response();
    }

    let email = &entry.user_email;

    let user = match db::upsert_user(&state.db, "email", email, Some(email), None).await {
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

// ── Email sending helper ──────────────────────────────────────────────────────

async fn send_email(state: &AppState, to: &str, magic_url: &str) -> Result<(), String> {
    let cfg = &state.config.auth.email;

    let smtp_host = cfg
        .smtp_host
        .as_deref()
        .ok_or_else(|| "smtp_host not configured".to_owned())?;
    let smtp_port = cfg.smtp_port.unwrap_or(587);
    let from_address = cfg
        .from_address
        .as_deref()
        .unwrap_or("noreply@lookwhatidid.xyz");

    let body = format!(
        "Click the link below to sign in to LookWhatIDid.\n\n{magic_url}\n\nThis link expires in {} minutes.",
        cfg.magic_link_ttl_minutes()
    );

    let email = Message::builder()
        .from(
            from_address
                .parse()
                .map_err(|e: lettre::address::AddressError| e.to_string())?,
        )
        .to(to
            .parse()
            .map_err(|e: lettre::address::AddressError| e.to_string())?)
        .subject("Your LookWhatIDid sign-in link")
        .header(ContentType::TEXT_PLAIN)
        .body(body)
        .map_err(|e| e.to_string())?;

    let transport_builder = if let Some(tls_hostname) = cfg.smtp_tls_hostname.as_deref() {
        // Connecting via tunnel: smtp_host is e.g. localhost but the TLS cert
        // is issued for tls_hostname (e.g. mail.lookwhatidid.xyz).
        let tls = TlsParameters::builder(tls_hostname.to_owned())
            .build_native()
            .map_err(|e| e.to_string())?;
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(smtp_host)
            .port(smtp_port)
            .tls(Tls::Required(tls))
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(smtp_host)
            .map_err(|e| e.to_string())?
            .port(smtp_port)
    };

    let transport_builder =
        if let (Some(user), Some(pass)) = (cfg.smtp_user.as_deref(), cfg.smtp_password.as_deref())
        {
            transport_builder.credentials(Credentials::new(user.to_owned(), pass.to_owned()))
        } else {
            transport_builder
        };

    let transport = transport_builder.build();

    transport
        .send(email)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}
