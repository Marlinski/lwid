//! Project-related HTTP API endpoints.
//!
//! Projects are mutable pointers to a content-addressed root [`Cid`],
//! authorised for writes by an Ed25519 public key. These endpoints allow
//! creating projects, fetching metadata, and updating the root CID with a
//! signed request.

use std::collections::BTreeSet;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use base64::prelude::*;
use chrono::Utc;
use tracing::info;

use lwid_common::auth::{self, AuthError};
use lwid_common::cid::Cid;
use lwid_common::limits::{parse_ttl, DEFAULT_TTL, TTL_CHOICES};
use lwid_common::manifest::Manifest;
use lwid_common::project::{Project, ProjectError};
use lwid_common::wire;

use crate::auth::OptionalUser;
use crate::db;
use crate::error::AppError;

use super::{clamp_ttl, tier_policy, AppState};

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

/// Convert a [`Project`] into a [`wire::ProjectResponse`].
fn project_to_response(project: &Project) -> wire::ProjectResponse {
    wire::ProjectResponse {
        id: project.id.clone(),
        root_cid: project.root_cid.as_ref().map(|c| c.to_string()),
        created_at: project.created_at.to_rfc3339(),
        updated_at: project.updated_at.to_rfc3339(),
        expires_at: project.expires_at.map(|t| t.to_rfc3339()),
    }
}

/// Collect all blob CIDs referenced by a manifest and its version chain.
///
/// For the current manifest this includes:
/// - The manifest blob itself (`manifest_cid`)
/// - All file blob CIDs listed in the manifest
///
/// If the manifest has a `parent_cid`, we walk the chain and accumulate CIDs
/// from every ancestor manifest as well. This ensures the project's `blob_cids`
/// set covers the entire history.
fn collect_all_blob_cids(
    state: &AppState,
    manifest_cid: &str,
    manifest: &Manifest,
) -> Result<BTreeSet<String>, AppError> {
    let mut cids = BTreeSet::new();

    // Add the manifest blob itself
    cids.insert(manifest_cid.to_string());

    // Add all file blobs from this manifest
    for file_cid in manifest.blob_cids() {
        cids.insert(file_cid.to_string());
    }

    // Walk the parent chain
    let mut parent = manifest.parent_cid.clone();
    while let Some(ref pcid_str) = parent {
        cids.insert(pcid_str.clone());

        let pcid = Cid::from_string(pcid_str)?;
        let data = state.blobs.get(&pcid).map_err(|e| {
            AppError::Internal(format!("failed to read parent manifest {pcid_str}: {e}"))
        })?;

        let parent_manifest: Manifest = serde_json::from_slice(&data).map_err(|e| {
            AppError::Internal(format!("failed to parse parent manifest {pcid_str}: {e}"))
        })?;

        for file_cid in parent_manifest.blob_cids() {
            cids.insert(file_cid.to_string());
        }

        parent = parent_manifest.parent_cid;
    }

    Ok(cids)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Create a new project.
///
/// The request body must contain a base64-encoded 32-byte Ed25519 public key
/// that will be authorised to update the project's root CID. An optional `ttl`
/// field controls the project's lifetime (defaults to `"7d"`).
///
/// # Errors
///
/// - `400 Bad Request` if the public key is not valid base64 or not 32 bytes,
///   or the TTL string is invalid.
/// - `500 Internal Server Error` on store failure.
pub async fn create_project(
    State(state): State<AppState>,
    user: OptionalUser,
    Json(req): Json<wire::CreateProjectRequest>,
) -> Result<(StatusCode, Json<wire::CreateProjectResponse>), AppError> {
    let policy = tier_policy(&state.config, &user);

    let pubkey = BASE64_STANDARD
        .decode(&req.write_pubkey)
        .map_err(|e| AppError::BadRequest(format!("invalid base64 for write_pubkey: {e}")))?;

    if pubkey.len() != 32 {
        return Err(AppError::BadRequest(format!(
            "write_pubkey must be 32 bytes, got {}",
            pubkey.len(),
        )));
    }

    let requested_ttl = req.ttl.as_deref().unwrap_or(DEFAULT_TTL);
    let ttl_str = clamp_ttl(requested_ttl, &policy.max_ttl);
    let expires_at =
        parse_ttl(ttl_str, Utc::now()).map_err(|e| AppError::BadRequest(e.to_string()))?;

    // Enforce max_projects quota (if limited and user is logged in).
    if policy.max_projects > 0 {
        if let Some(ref u) = user.0 {
            let all_ids = state.projects.list().map_err(|e| {
                AppError::Internal(format!("failed to list projects: {e}"))
            })?;
            let count = db::count_live_projects(&state.db, &u.id, &all_ids)
                .await
                .map_err(|e| AppError::Internal(format!("failed to count projects: {e}")))?;
            if count >= policy.max_projects {
                return Err(AppError::PayloadTooLarge(format!(
                    "project quota exceeded: limit={}, current={}",
                    policy.max_projects, count,
                )));
            }
        }
    }

    let created_with = req.client_version.unwrap_or_else(|| env!("LWID_VERSION").to_string());
    let project = state.projects.create(&pubkey, expires_at, req.store_token, Some(created_with.clone()))?;

    // Record ownership if the user is logged in.
    if let Some(ref u) = user.0 {
        if let Err(e) = db::set_project_owner(&state.db, &project.id, &u.id).await {
            tracing::warn!(project_id = %project.id, error = %e, "failed to record project owner");
        }
    }

    info!(
        project_id = %project.id,
        ttl = ttl_str,
        expires_at = ?project.expires_at,
        created_with = %created_with,
        "created project",
    );

    Ok((
        StatusCode::CREATED,
        Json(wire::CreateProjectResponse {
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
) -> Result<Json<wire::ProjectResponse>, AppError> {
    let project = state.projects.get(&id)?;
    Ok(Json(project_to_response(&project)))
}

/// Update the root CID of a project.
///
/// The caller must provide a valid Ed25519 signature (base64-encoded) over the
/// UTF-8 bytes of the new root CID string, signed with the project's write key.
///
/// The manifest blob is fetched and parsed to:
/// - Validate total file size against the project size limit
/// - Extract all blob CIDs for garbage-collection tracking
///
/// # Errors
///
/// - `404 Not Found` if the project does not exist.
/// - `400 Bad Request` if the signature is not valid base64, the CID is
///   invalid, or the manifest is malformed.
/// - `401 Unauthorized` if signature verification fails.
/// - `413 Payload Too Large` if total file size exceeds the limit.
/// - `500 Internal Server Error` on store failure.
pub async fn update_root(
    State(state): State<AppState>,
    user: OptionalUser,
    Path(id): Path<String>,
    Json(req): Json<wire::UpdateRootRequest>,
) -> Result<Json<wire::ProjectResponse>, AppError> {
    let policy = tier_policy(&state.config, &user);

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

    // Fetch the manifest blob and parse it.
    let manifest_data = state.blobs.get(&cid).map_err(|e| {
        AppError::BadRequest(format!("manifest blob not found for CID {}: {e}", req.root_cid))
    })?;

    let manifest: Manifest = serde_json::from_slice(&manifest_data).map_err(|e| {
        AppError::BadRequest(format!("invalid manifest JSON: {e}"))
    })?;

    // Validate total project size against the tier policy.
    let total_size = manifest.total_size();
    let max_project_size = policy.max_project_size as u64;
    if total_size > max_project_size {
        return Err(AppError::PayloadTooLarge(format!(
            "total file size ({total_size} bytes) exceeds project limit ({max_project_size} bytes)",
        )));
    }

    // Collect all blob CIDs (files + manifests across the version chain).
    let blob_cids = collect_all_blob_cids(&state, &req.root_cid, &manifest)?;

    // Persist the update.
    let updated = state.projects.update_root(&id, cid, blob_cids)?;

    info!(
        project_id = %id,
        root_cid = %req.root_cid,
        total_size,
        files = manifest.files.len(),
        "updated project root",
    );

    Ok(Json(project_to_response(&updated)))
}

/// Extend (or change) the TTL of a project.
///
/// The caller must provide a valid Ed25519 signature (base64-encoded) over the
/// UTF-8 bytes of the new TTL string, signed with the project's write key.
///
/// # Errors
///
/// - `400 Bad Request` if the TTL string is not one of the valid choices, or
///   the signature is not valid base64.
/// - `401 Unauthorized` if signature verification fails.
/// - `404 Not Found` if the project does not exist.
/// - `500 Internal Server Error` on store failure.
pub async fn extend_ttl(
    State(state): State<AppState>,
    user: OptionalUser,
    Path(id): Path<String>,
    Json(body): Json<wire::ExtendTtlRequest>,
) -> Result<Json<wire::ProjectResponse>, AppError> {
    // Validate that the TTL string is one of the accepted choices.
    if !TTL_CHOICES.contains(&body.ttl.as_str()) {
        return Err(AppError::BadRequest(format!(
            "invalid TTL '{}': expected one of {}",
            body.ttl,
            TTL_CHOICES.join(", "),
        )));
    }

    let policy = tier_policy(&state.config, &user);

    // Clamp TTL to what this tier allows.
    let effective_ttl = clamp_ttl(&body.ttl, &policy.max_ttl).to_owned();

    // Fetch the project to obtain the write pubkey.
    let project = state.projects.get(&id)?;

    // Decode the signature from base64.
    let signature_bytes = BASE64_STANDARD
        .decode(&body.signature)
        .map_err(|e| AppError::BadRequest(format!("invalid base64 for signature: {e}")))?;

    // Verify the signature over the TTL string bytes (the originally-requested TTL).
    auth::verify_signature(
        &project.write_pubkey,
        body.ttl.as_bytes(),
        &signature_bytes,
    )?;

    // Parse the (possibly clamped) TTL into an expiry timestamp.
    let expires_at =
        parse_ttl(&effective_ttl, Utc::now()).map_err(|e| AppError::BadRequest(e.to_string()))?;

    // Persist the updated expiry.
    let updated = state.projects.update_expiry(&id, expires_at)?;

    info!(
        project_id = %id,
        ttl = %effective_ttl,
        expires_at = ?updated.expires_at,
        "extended project TTL",
    );

    Ok(Json(project_to_response(&updated)))
}

/// Delete a project.
///
/// The caller must provide a valid Ed25519 signature (base64-encoded) over the
/// UTF-8 bytes of the project ID, signed with the project's write key.
///
/// # Errors
///
/// - `404 Not Found` if the project does not exist.
/// - `400 Bad Request` if the signature is not valid base64.
/// - `401 Unauthorized` if signature verification fails.
/// - `500 Internal Server Error` on store failure.
pub async fn delete_project(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<wire::DeleteProjectRequest>,
) -> Result<StatusCode, AppError> {
    // Fetch the project to obtain the write pubkey.
    let project = state.projects.get(&id)?;

    // Decode the signature from base64.
    let signature_bytes = BASE64_STANDARD
        .decode(&body.signature)
        .map_err(|e| AppError::BadRequest(format!("invalid base64 for signature: {e}")))?;

    // Verify the signature over the project ID string bytes.
    auth::verify_signature(
        &project.write_pubkey,
        id.as_bytes(),
        &signature_bytes,
    )?;

    // Delete the project.
    state.projects.delete(&id)?;

    // Clean up store KV entries for this project.
    if let Err(e) = state.kv.delete_all(&id) {
        tracing::warn!(project_id = %id, error = %e, "failed to clean up store entries");
    }

    info!(project_id = %id, "deleted project");

    Ok(StatusCode::NO_CONTENT)
}
