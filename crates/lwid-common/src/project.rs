//! Project metadata management.
//!
//! A **project** is a named pointer to a content-addressed root [`Cid`],
//! authorised for writes by an Ed25519 public key. Projects start with no
//! content (`root_cid` is `None`) and acquire a root CID when the first
//! version is published via [`ProjectStore::update_root`].

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::cid::Cid;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by project-store operations.
#[derive(Debug, Error)]
pub enum ProjectError {
    /// An underlying I/O operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The requested project does not exist.
    #[error("project not found: {id}")]
    NotFound { id: String },

    /// JSON (de)serialization failed.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Project struct
// ---------------------------------------------------------------------------

/// Metadata for a single project on the lookwhatidid platform.
///
/// A project links an opaque identifier to an optional content-addressed root
/// [`Cid`] and the Ed25519 public key that is authorised to update it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    /// Unique project identifier (nanoid, 12 characters).
    pub id: String,

    /// The current root CID of the project's content tree.
    ///
    /// `None` when the project has just been created and no content has been
    /// published yet.
    pub root_cid: Option<Cid>,

    /// 32-byte Ed25519 public key that authorises writes to this project.
    pub write_pubkey: Vec<u8>,

    /// Timestamp when the project was created.
    pub created_at: DateTime<Utc>,

    /// Timestamp of the last metadata update (e.g. root CID change).
    pub updated_at: DateTime<Utc>,

    /// When this project expires and becomes eligible for deletion.
    ///
    /// `None` means the project never expires. Backward-compatible: old
    /// project files without this field deserialize as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,

    /// All blob CIDs currently referenced by this project (file blobs +
    /// manifest blobs from the version chain).
    ///
    /// Used by the reaper to garbage-collect orphaned blobs when the project
    /// is deleted.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub blob_cids: BTreeSet<String>,

    /// SHA-256 store authentication token (base64-encoded).
    /// Clients derive this from the read key to prove store access without revealing the key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_token: Option<String>,

    /// lwid version that created this project (client_version from the create request).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_with: Option<String>,
}

impl Project {
    /// Returns `true` if this project has expired as of `now`.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|exp| now >= exp)
    }
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Storage backend for [`Project`] metadata.
///
/// Implementations must be safe to share across threads.
pub trait ProjectStore: Send + Sync {
    /// Create a new project with the given Ed25519 public key, optional
    /// expiry time, optional store authentication token, and optional client version.
    fn create(
        &self,
        write_pubkey: &[u8],
        expires_at: Option<DateTime<Utc>>,
        store_token: Option<String>,
        created_with: Option<String>,
    ) -> Result<Project, ProjectError>;

    /// Retrieve a project by its identifier.
    ///
    /// Returns [`ProjectError::NotFound`] if no project with `id` exists.
    fn get(&self, id: &str) -> Result<Project, ProjectError>;

    /// Update the root CID of an existing project, along with the set of
    /// blob CIDs it references.
    ///
    /// The `updated_at` timestamp is refreshed automatically.
    fn update_root(
        &self,
        id: &str,
        root_cid: Cid,
        blob_cids: BTreeSet<String>,
    ) -> Result<Project, ProjectError>;

    /// Delete a project by its identifier. Returns the deleted project.
    fn delete(&self, id: &str) -> Result<Project, ProjectError>;

    /// Update the expiry timestamp of an existing project.
    ///
    /// The `updated_at` timestamp is refreshed automatically.
    fn update_expiry(
        &self,
        id: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<Project, ProjectError>;

    /// List the identifiers of all known projects.
    fn list(&self) -> Result<Vec<String>, ProjectError>;
}

// ---------------------------------------------------------------------------
// Filesystem implementation
// ---------------------------------------------------------------------------

/// A [`ProjectStore`] backed by the local filesystem.
///
/// Each project is stored as a JSON file named `{id}.json` under `base_dir`.
pub struct FsProjectStore {
    base_dir: PathBuf,
}

impl FsProjectStore {
    /// Create a new filesystem project store rooted at `base_dir`.
    ///
    /// The directory (and any missing parents) is created if it does not
    /// already exist.
    pub fn new(base_dir: PathBuf) -> Result<Self, ProjectError> {
        fs::create_dir_all(&base_dir)?;
        Ok(Self { base_dir })
    }

    /// Return the path to the JSON file for a given project ID.
    fn project_path(&self, id: &str) -> PathBuf {
        self.base_dir.join(format!("{id}.json"))
    }

    /// Ensure the base directory exists (re-create if deleted at runtime).
    fn ensure_dir(&self) -> Result<(), ProjectError> {
        fs::create_dir_all(&self.base_dir)?;
        Ok(())
    }
}

impl ProjectStore for FsProjectStore {
    fn create(
        &self,
        write_pubkey: &[u8],
        expires_at: Option<DateTime<Utc>>,
        store_token: Option<String>,
        created_with: Option<String>,
    ) -> Result<Project, ProjectError> {
        let id = nanoid::nanoid!(12);
        let now = Utc::now();

        let project = Project {
            id,
            root_cid: None,
            write_pubkey: write_pubkey.to_vec(),
            created_at: now,
            updated_at: now,
            expires_at,
            blob_cids: BTreeSet::new(),
            store_token,
            created_with,
        };

        let json = serde_json::to_string_pretty(&project)?;
        self.ensure_dir()?;
        fs::write(self.project_path(&project.id), json)?;

        Ok(project)
    }

    fn get(&self, id: &str) -> Result<Project, ProjectError> {
        let path = self.project_path(id);

        if !path.exists() {
            return Err(ProjectError::NotFound { id: id.to_owned() });
        }

        let json = fs::read_to_string(&path)?;
        let project: Project = serde_json::from_str(&json)?;
        Ok(project)
    }

    fn update_root(
        &self,
        id: &str,
        root_cid: Cid,
        blob_cids: BTreeSet<String>,
    ) -> Result<Project, ProjectError> {
        let mut project = self.get(id)?;

        project.root_cid = Some(root_cid);
        project.blob_cids = blob_cids;
        project.updated_at = Utc::now();

        let json = serde_json::to_string_pretty(&project)?;
        self.ensure_dir()?;
        fs::write(self.project_path(id), json)?;

        Ok(project)
    }

    fn delete(&self, id: &str) -> Result<Project, ProjectError> {
        let project = self.get(id)?;
        let path = self.project_path(id);
        fs::remove_file(&path)?;
        Ok(project)
    }

    fn update_expiry(
        &self,
        id: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<Project, ProjectError> {
        let mut project = self.get(id)?;

        project.expires_at = expires_at;
        project.updated_at = Utc::now();

        let json = serde_json::to_string_pretty(&project)?;
        self.ensure_dir()?;
        fs::write(self.project_path(id), json)?;

        Ok(project)
    }

    fn list(&self) -> Result<Vec<String>, ProjectError> {
        self.ensure_dir()?;
        let mut ids = Vec::new();

        for entry in fs::read_dir(&self.base_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    ids.push(stem.to_owned());
                }
            }
        }

        Ok(ids)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create a temporary [`FsProjectStore`].
    fn tmp_store() -> (FsProjectStore, TempDir) {
        let dir = TempDir::new().expect("failed to create temp dir");
        let store = FsProjectStore::new(dir.path().join("projects"))
            .expect("failed to create FsProjectStore");
        (store, dir)
    }

    /// Dummy 32-byte Ed25519 public key for testing.
    fn test_pubkey() -> Vec<u8> {
        vec![0xAB; 32]
    }

    #[test]
    fn create_then_get_roundtrip() {
        let (store, _dir) = tmp_store();
        let pubkey = test_pubkey();

        let created = store
            .create(&pubkey, None, None, None)
            .expect("create should succeed");

        assert_eq!(created.id.len(), 12, "project ID should be 12 characters");
        assert!(
            created.root_cid.is_none(),
            "new project should have no root CID"
        );
        assert_eq!(created.write_pubkey, pubkey);
        assert!(created.expires_at.is_none());
        assert!(created.blob_cids.is_empty());

        let fetched = store.get(&created.id).expect("get should succeed");

        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.root_cid, created.root_cid);
        assert_eq!(fetched.write_pubkey, created.write_pubkey);
        assert_eq!(fetched.created_at, created.created_at);
        assert_eq!(fetched.updated_at, created.updated_at);
    }

    #[test]
    fn create_with_expiry() {
        let (store, _dir) = tmp_store();
        let pubkey = test_pubkey();
        let expires = Utc::now() + chrono::Duration::hours(1);

        let created = store
            .create(&pubkey, Some(expires), None, None)
            .expect("create should succeed");

        assert_eq!(created.expires_at, Some(expires));
        assert!(!created.is_expired(Utc::now()));

        // Simulate time passing
        let future = expires + chrono::Duration::seconds(1);
        assert!(created.is_expired(future));
    }

    #[test]
    fn get_nonexistent_returns_not_found() {
        let (store, _dir) = tmp_store();

        let err = store
            .get("does-not-exist")
            .expect_err("get should fail for missing project");

        match err {
            ProjectError::NotFound { ref id } => {
                assert_eq!(id, "does-not-exist");
            }
            _ => panic!("expected ProjectError::NotFound, got: {err:?}"),
        }
    }

    #[test]
    fn update_root_works_and_updates_timestamp() {
        let (store, _dir) = tmp_store();
        let pubkey = test_pubkey();

        let created = store
            .create(&pubkey, None, None, None)
            .expect("create should succeed");
        assert!(created.root_cid.is_none());

        let cid = Cid::from_bytes(b"project content v1");
        let blobs: BTreeSet<String> = ["bafk1".to_string(), "bafk2".to_string()]
            .into_iter()
            .collect();
        let updated = store
            .update_root(&created.id, cid.clone(), blobs.clone())
            .expect("update_root should succeed");

        assert_eq!(updated.root_cid.as_ref(), Some(&cid));
        assert_eq!(updated.blob_cids, blobs);
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.write_pubkey, created.write_pubkey);
        assert_eq!(updated.created_at, created.created_at);
        assert!(
            updated.updated_at >= created.updated_at,
            "updated_at should be >= created_at after update",
        );

        // Verify persistence.
        let fetched = store.get(&created.id).expect("get after update");
        assert_eq!(fetched.root_cid.as_ref(), Some(&cid));
        assert_eq!(fetched.blob_cids, blobs);
    }

    #[test]
    fn delete_removes_project() {
        let (store, _dir) = tmp_store();
        let pubkey = test_pubkey();

        let created = store
            .create(&pubkey, None, None, None)
            .expect("create should succeed");
        let deleted = store.delete(&created.id).expect("delete should succeed");
        assert_eq!(deleted.id, created.id);

        let err = store
            .get(&created.id)
            .expect_err("get after delete should fail");
        assert!(matches!(err, ProjectError::NotFound { .. }));
    }

    #[test]
    fn list_returns_created_projects() {
        let (store, _dir) = tmp_store();
        let pubkey = test_pubkey();

        let p1 = store.create(&pubkey, None, None, None).expect("create p1");
        let p2 = store.create(&pubkey, None, None, None).expect("create p2");
        let p3 = store.create(&pubkey, None, None, None).expect("create p3");

        let mut ids = store.list().expect("list should succeed");
        ids.sort();

        let mut expected = vec![p1.id, p2.id, p3.id];
        expected.sort();

        assert_eq!(ids, expected);
    }
}
