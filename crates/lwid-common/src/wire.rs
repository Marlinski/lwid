//! Wire types shared between client and server.
//!
//! These represent the JSON shapes exchanged over the HTTP API.

use serde::{Deserialize, Serialize};

/// Request body for `POST /api/projects`.
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateProjectRequest {
    pub write_pubkey: String,
    /// Optional TTL string: `"1h"`, `"1d"`, `"7d"`, `"30d"`, or `"never"`.
    /// Defaults to `"7d"` if omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>,
    /// SHA-256 hash of the read key, base64-encoded. Used to authenticate store operations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_token: Option<String>,
}

/// Response body for `POST /api/projects`.
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateProjectResponse {
    pub project_id: String,
}

/// Response body for `GET /api/projects/{id}`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectResponse {
    pub id: String,
    pub root_cid: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// ISO 8601 expiry timestamp, or `null` if the project never expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// Request body for `PUT /api/projects/{id}/root`.
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateRootRequest {
    pub root_cid: String,
    pub signature: String,
}

/// Request body for `PUT /api/projects/{id}/ttl`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ExtendTtlRequest {
    pub ttl: String,
    /// Base64-encoded Ed25519 signature over the TTL string bytes.
    pub signature: String,
}

/// Response body for `POST /api/blobs`.
#[derive(Debug, Serialize, Deserialize)]
pub struct UploadBlobResponse {
    pub cid: String,
}

/// Response body for `GET /api/projects/{id}/store` (list keys).
#[derive(Debug, Serialize, Deserialize)]
pub struct StoreListResponse {
    pub keys: Vec<String>,
    pub total_size: u64,
}
