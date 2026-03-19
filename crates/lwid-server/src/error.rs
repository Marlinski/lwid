//! Unified application error type.
//!
//! [`AppError`] wraps every domain error in the crate and implements
//! [`axum::response::IntoResponse`] so handlers can use `Result<T, AppError>`
//! directly.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::ConfigError;
use lwid_common::cid::CidError;
use lwid_common::store::StoreError;

// ---------------------------------------------------------------------------
// Error enum
// ---------------------------------------------------------------------------

/// Unified error type for all HTTP handlers.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// Blob-store error (I/O failure or missing blob).
    #[error(transparent)]
    Store(#[from] StoreError),

    /// CID validation / parsing error.
    #[error(transparent)]
    Cid(#[from] CidError),

    /// Configuration loading error.
    #[error(transparent)]
    Config(#[from] ConfigError),

    /// Generic "not found".
    #[error("not found: {0}")]
    NotFound(String),

    /// Invalid input from the client.
    #[error("bad request: {0}")]
    BadRequest(String),

    /// Signature or authentication failure.
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    /// Upload exceeds the configured size limit.
    #[error("payload too large: {0}")]
    PayloadTooLarge(String),

    /// Catch-all for unexpected internal errors.
    #[error("internal error: {0}")]
    Internal(String),
}

// ---------------------------------------------------------------------------
// IntoResponse
// ---------------------------------------------------------------------------

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::Store(StoreError::NotFound { .. }) => {
                (StatusCode::NOT_FOUND, self.to_string())
            }
            AppError::Store(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            AppError::Cid(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::Config(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, self.to_string()),
            AppError::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, self.to_string()),
            AppError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        let body = serde_json::json!({ "error": message });
        (status, axum::Json(body)).into_response()
    }
}
