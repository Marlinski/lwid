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
    /// lwid version of the client that created this project (e.g. git short hash).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_version: Option<String>,
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
    pub keys: Vec<StoreKeyEntry>,
    pub total_size: u64,
}

/// A single entry in the store key listing.
#[derive(Debug, Serialize, Deserialize)]
pub struct StoreKeyEntry {
    pub key: String,
    pub size: u64,
}

/// Request body for `DELETE /api/projects/{id}`.
#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteProjectRequest {
    /// Base64-encoded Ed25519 signature over the project ID string bytes.
    pub signature: String,
}

/// Response body for `GET /api/version`.
#[derive(Debug, Serialize, Deserialize)]
pub struct VersionResponse {
    pub version: String,
}

/// Response body for `GET /auth/me`.
#[derive(Debug, Serialize, Deserialize)]
pub struct UserResponse {
    pub id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    /// "anonymous" | "free" | "pro"
    pub tier: String,
    /// e.g. ["github", "google"]
    pub enabled_providers: Vec<String>,
}

/// Request body for `POST /auth/magic`.
#[derive(Debug, Serialize, Deserialize)]
pub struct MagicLinkRequest {
    pub email: String,
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

/// Response body for `GET /api/manifest`.
///
/// Unauthenticated — returns static server configuration that the frontend
/// can use to conditionally render UI (sign-in button, quota indicators, etc.)
/// before any user interaction.
#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestResponse {
    pub auth: ManifestAuth,
    pub policy: ManifestPolicy,
}

/// Auth section of the manifest.
#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestAuth {
    /// True if at least one auth provider is configured.
    pub enabled: bool,
    /// Names of the configured providers, e.g. `["github", "google", "email"]`.
    pub providers: Vec<String>,
}

/// Policy section of the manifest — all three tiers exposed so the frontend
/// can show upgrade incentives and pre-validate sizes client-side.
#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestPolicy {
    pub anonymous: ManifestTierPolicy,
    pub free: ManifestTierPolicy,
    pub pro: ManifestTierPolicy,
}

/// Quota limits for a single tier.
#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestTierPolicy {
    pub max_blob_size: usize,
    pub max_project_size: usize,
    pub max_store_total: usize,
    /// Maximum TTL string, e.g. `"7d"` or `"never"`.
    pub max_ttl: String,
    /// Maximum number of live projects. `0` means unlimited.
    pub max_projects: usize,
}
