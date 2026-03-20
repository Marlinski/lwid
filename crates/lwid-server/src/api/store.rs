//! Store-related HTTP API endpoints.
//!
//! The store is a mutable, non-versioned key-value / blob store scoped per
//! project. Access is authenticated via an `X-Store-Token` header whose value
//! must match the `store_token` stored on the project at creation time.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use subtle::ConstantTimeEq;
use tracing::info;

use lwid_common::kv::KvError;
use lwid_common::limits::MAX_STORE_VALUE_SIZE;
use lwid_common::wire;

use crate::error::AppError;

use super::AppState;

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

impl From<KvError> for AppError {
    fn from(err: KvError) -> Self {
        match err {
            KvError::NotFound { project_id, key } => {
                AppError::NotFound(format!("store key not found: project={project_id}, key={key}"))
            }
            KvError::InvalidKey { key, reason } => {
                AppError::BadRequest(format!("invalid store key \"{key}\": {reason}"))
            }
            KvError::QuotaExceeded {
                project_id,
                limit,
                current,
            } => AppError::PayloadTooLarge(format!(
                "store quota exceeded for project {project_id}: limit={limit}, current={current}"
            )),
            KvError::Io(e) => AppError::Internal(format!("store I/O error: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Header name for the store authentication token.
const STORE_TOKEN_HEADER: &str = "x-store-token";

/// Extract and verify the store token from the request headers against the
/// project's stored token. Returns an error if the token is missing, the
/// project has no store token, or the token doesn't match.
fn verify_store_token(
    headers: &HeaderMap,
    project_store_token: &Option<String>,
) -> Result<(), AppError> {
    let provided = headers
        .get(STORE_TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("missing X-Store-Token header".to_owned()))?;

    let expected = project_store_token.as_deref().ok_or_else(|| {
        AppError::Unauthorized("store not enabled for this project".to_owned())
    })?;

    // Constant-time comparison to prevent timing attacks.
    let provided_bytes = provided.as_bytes();
    let expected_bytes = expected.as_bytes();

    if provided_bytes.len() != expected_bytes.len()
        || provided_bytes.ct_eq(expected_bytes).unwrap_u8() != 1
    {
        return Err(AppError::Unauthorized(
            "invalid store token".to_owned(),
        ));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `PUT /api/projects/{id}/store/{*key}` — write a value.
///
/// The request body is treated as raw bytes. The `X-Store-Token` header must
/// match the project's stored token.
pub async fn put_value(
    State(state): State<AppState>,
    Path((id, key)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let project = state.projects.get(&id)?;
    verify_store_token(&headers, &project.store_token)?;

    if body.len() > MAX_STORE_VALUE_SIZE {
        return Err(AppError::PayloadTooLarge(format!(
            "store value size {} exceeds maximum of {} bytes",
            body.len(),
            MAX_STORE_VALUE_SIZE,
        )));
    }

    state.kv.put(&id, &key, &body)?;

    info!(
        project_id = %id,
        key = %key,
        size = body.len(),
        "store: put value",
    );

    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/projects/{id}/store/{*key}` — read a value.
///
/// Returns raw bytes with `Content-Type: application/octet-stream`.
pub async fn get_value(
    State(state): State<AppState>,
    Path((id, key)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let project = state.projects.get(&id)?;
    verify_store_token(&headers, &project.store_token)?;

    let data = state.kv.get(&id, &key)?;

    let response = (
        [(CONTENT_TYPE, "application/octet-stream")],
        data,
    )
        .into_response();

    Ok(response)
}

/// `DELETE /api/projects/{id}/store/{*key}` — delete a value.
pub async fn delete_value(
    State(state): State<AppState>,
    Path((id, key)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    let project = state.projects.get(&id)?;
    verify_store_token(&headers, &project.store_token)?;

    state.kv.delete(&id, &key)?;

    info!(
        project_id = %id,
        key = %key,
        "store: deleted value",
    );

    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/projects/{id}/store` — list all keys and total size.
pub async fn list_keys(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<wire::StoreListResponse>, AppError> {
    let project = state.projects.get(&id)?;
    verify_store_token(&headers, &project.store_token)?;

    let keys = state.kv.list_keys(&id)?;
    let total_size = state.kv.total_size(&id)?;

    Ok(Json(wire::StoreListResponse { keys, total_size }))
}
