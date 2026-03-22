//! Blob-related HTTP API endpoints.
//!
//! Blobs are immutable, content-addressed binary objects stored and retrieved
//! by their [`Cid`]. Because the identifier is derived from the content, blobs
//! are inherently cacheable forever.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use lwid_common::cid::Cid;
use crate::auth::OptionalUser;
use crate::error::AppError;

use super::{tier_policy, AppState};

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// Response body for a successful single-blob upload.
#[derive(Debug, Serialize, Deserialize)]
pub struct UploadResponse {
    /// The CID of the stored blob (multibase base32lower).
    pub cid: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Upload a single blob.
///
/// The request body is treated as raw binary. The blob is stored in the
/// content-addressed store and its CID is returned.
///
/// # Errors
///
/// - `413 Payload Too Large` if the body exceeds `config.server.max_blob_size`.
/// - `500 Internal Server Error` on store I/O failure.
pub async fn upload_blob(
    State(state): State<AppState>,
    user: OptionalUser,
    body: Bytes,
) -> Result<Json<UploadResponse>, AppError> {
    let policy = tier_policy(&state.config, &user);
    let max = policy.max_blob_size;
    if body.len() > max {
        return Err(AppError::PayloadTooLarge(format!(
            "blob size {} exceeds maximum of {} bytes",
            body.len(),
            max,
        )));
    }

    let cid = state.blobs.put(&body)?;

    Ok(Json(UploadResponse {
        cid: cid.to_string(),
    }))
}

/// Retrieve a blob by CID.
///
/// Returns the raw bytes with `Content-Type: application/octet-stream` and an
/// aggressive `Cache-Control` header — content-addressed data is immutable by
/// definition.
///
/// # Errors
///
/// - `400 Bad Request` if the CID string is malformed.
/// - `404 Not Found` if the blob does not exist.
/// - `500 Internal Server Error` on store I/O failure.
pub async fn get_blob(
    State(state): State<AppState>,
    Path(cid_str): Path<String>,
) -> Result<Response, AppError> {
    let cid = Cid::from_string(&cid_str)?;
    let data = state.blobs.get(&cid)?;

    let response = (
        [
            (CONTENT_TYPE, "application/octet-stream"),
            (CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        data,
    )
        .into_response();

    Ok(response)
}

/// Check whether a blob exists (HEAD request).
///
/// Returns `200 OK` if the blob is present, or `404 Not Found` otherwise.
///
/// # Errors
///
/// - `400 Bad Request` if the CID string is malformed.
/// - `500 Internal Server Error` on store I/O failure.
pub async fn head_blob(
    State(state): State<AppState>,
    Path(cid_str): Path<String>,
) -> Result<StatusCode, AppError> {
    let cid = Cid::from_string(&cid_str)?;

    if state.blobs.exists(&cid)? {
        Ok(StatusCode::OK)
    } else {
        Err(AppError::NotFound(format!("blob not found: {cid_str}")))
    }
}
