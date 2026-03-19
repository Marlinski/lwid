//! Wire types shared between client and server.
//!
//! These represent the JSON shapes exchanged over the HTTP API.

use serde::{Deserialize, Serialize};

/// Request body for `POST /api/projects`.
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateProjectRequest {
    pub write_pubkey: String,
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
}

/// Request body for `PUT /api/projects/{id}/root`.
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateRootRequest {
    pub root_cid: String,
    pub signature: String,
}

/// Response body for `POST /api/blobs`.
#[derive(Debug, Serialize, Deserialize)]
pub struct UploadBlobResponse {
    pub cid: String,
}
