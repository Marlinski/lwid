//! Project-related HTTP API endpoints.
//!
//! Projects are mutable pointers to a content-addressed root [`Cid`],
//! authorised for writes by an Ed25519 public key. These endpoints allow
//! creating projects, fetching metadata, and updating the root CID with a
//! signed request.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use base64::prelude::*;
use serde::{Deserialize, Serialize};

use lwid_common::auth::{self, AuthError};
use lwid_common::cid::Cid;
use crate::error::AppError;
use lwid_common::project::{Project, ProjectError};

use super::AppState;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// Request body for creating a new project.
#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    /// Base64-encoded 32-byte Ed25519 public key.
    pub write_pubkey: String,
}

/// Response body after successfully creating a project.
#[derive(Debug, Serialize)]
pub struct CreateResponse {
    /// The unique project identifier.
    pub project_id: String,
}

/// Response body representing project metadata.
#[derive(Debug, Serialize)]
pub struct ProjectResponse {
    /// Unique project identifier.
    pub id: String,
    /// The current root CID, if any.
    pub root_cid: Option<String>,
    /// ISO 8601 timestamp when the project was created.
    pub created_at: String,
    /// ISO 8601 timestamp of the last update.
    pub updated_at: String,
}

/// Request body for updating a project's root CID.
#[derive(Debug, Deserialize)]
pub struct UpdateRootRequest {
    /// The new root CID string.
    pub root_cid: String,
    /// Base64-encoded Ed25519 signature over the `root_cid` string (UTF-8 bytes).
    pub signature: String,
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

impl From<ProjectError> for AppError {
    fn from(err: ProjectError) -> Self {
        match err {
            ProjectError::NotFound { id } => {
                AppError::NotFound(format!("project not found: {id}"))
            }
            ProjectError::Io(e) => AppError::Internal(format!("project I/O error: {e}")),
            ProjectError::Serde(e) => {
                AppError::Internal(format!("project serialization error: {e}"))
            }
        }
    }
}

impl From<AuthError> for AppError {
    fn from(err: AuthError) -> Self {
        AppError::Unauthorized(err.to_string())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a [`Project`] into a [`ProjectResponse`], omitting the write pubkey.
fn project_to_response(project: &Project) -> ProjectResponse {
    ProjectResponse {
        id: project.id.clone(),
        root_cid: project.root_cid.as_ref().map(|c| c.to_string()),
        created_at: project.created_at.to_rfc3339(),
        updated_at: project.updated_at.to_rfc3339(),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Create a new project.
///
/// The request body must contain a base64-encoded 32-byte Ed25519 public key
/// that will be authorised to update the project's root CID.
///
/// # Errors
///
/// - `400 Bad Request` if the public key is not valid base64 or not 32 bytes.
/// - `500 Internal Server Error` on store failure.
pub async fn create_project(
    State(state): State<AppState>,
    Json(req): Json<CreateRequest>,
) -> Result<(StatusCode, Json<CreateResponse>), AppError> {
    let pubkey = BASE64_STANDARD
        .decode(&req.write_pubkey)
        .map_err(|e| AppError::BadRequest(format!("invalid base64 for write_pubkey: {e}")))?;

    if pubkey.len() != 32 {
        return Err(AppError::BadRequest(format!(
            "write_pubkey must be 32 bytes, got {}",
            pubkey.len(),
        )));
    }

    let project = state.projects.create(&pubkey)?;

    Ok((
        StatusCode::CREATED,
        Json(CreateResponse {
            project_id: project.id,
        }),
    ))
}

/// Retrieve project metadata by ID.
///
/// The write public key is intentionally omitted from the response.
///
/// # Errors
///
/// - `404 Not Found` if no project with the given ID exists.
/// - `500 Internal Server Error` on store failure.
pub async fn get_project(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProjectResponse>, AppError> {
    let project = state.projects.get(&id)?;
    Ok(Json(project_to_response(&project)))
}

/// Update the root CID of a project.
///
/// The caller must provide a valid Ed25519 signature (base64-encoded) over the
/// UTF-8 bytes of the new root CID string, signed with the project's write key.
///
/// # Errors
///
/// - `404 Not Found` if the project does not exist.
/// - `400 Bad Request` if the signature is not valid base64 or the CID is invalid.
/// - `401 Unauthorized` if signature verification fails.
/// - `500 Internal Server Error` on store failure.
pub async fn update_root(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRootRequest>,
) -> Result<Json<ProjectResponse>, AppError> {
    // Fetch the project to obtain the write pubkey.
    let project = state.projects.get(&id)?;

    // Decode the signature from base64.
    let signature_bytes = BASE64_STANDARD
        .decode(&req.signature)
        .map_err(|e| AppError::BadRequest(format!("invalid base64 for signature: {e}")))?;

    // Verify the signature over the root CID string bytes.
    auth::verify_signature(
        &project.write_pubkey,
        req.root_cid.as_bytes(),
        &signature_bytes,
    )?;

    // Parse and validate the CID.
    let cid = Cid::from_string(&req.root_cid)?;

    // Persist the update.
    let updated = state.projects.update_root(&id, cid)?;

    Ok(Json(project_to_response(&updated)))
}
