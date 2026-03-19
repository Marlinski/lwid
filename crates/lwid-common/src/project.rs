//! Project metadata management.
//!
//! A **project** is a named pointer to a content-addressed root [`Cid`],
//! authorised for writes by an Ed25519 public key. Projects start with no
//! content (`root_cid` is `None`) and acquire a root CID when the first
//! version is published via [`ProjectStore::update_root`].

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
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Storage backend for [`Project`] metadata.
///
/// Implementations must be safe to share across threads.
pub trait ProjectStore: Send + Sync {
    /// Create a new project with the given Ed25519 public key.
    ///
    /// A unique project ID is generated automatically and the project is
    /// persisted with `root_cid` set to `None`.
    fn create(&self, write_pubkey: &[u8]) -> Result<Project, ProjectError>;

    /// Retrieve a project by its identifier.
    ///
    /// Returns [`ProjectError::NotFound`] if no project with `id` exists.
    fn get(&self, id: &str) -> Result<Project, ProjectError>;

    /// Update the root CID of an existing project.
    ///
    /// The `updated_at` timestamp is refreshed automatically.
    fn update_root(&self, id: &str, root_cid: Cid) -> Result<Project, ProjectError>;

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
}

impl ProjectStore for FsProjectStore {
    fn create(&self, write_pubkey: &[u8]) -> Result<Project, ProjectError> {
        let id = nanoid::nanoid!(12);
        let now = Utc::now();

        let project = Project {
            id,
            root_cid: None,
            write_pubkey: write_pubkey.to_vec(),
            created_at: now,
            updated_at: now,
        };

        let json = serde_json::to_string_pretty(&project)?;
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

    fn update_root(&self, id: &str, root_cid: Cid) -> Result<Project, ProjectError> {
        let mut project = self.get(id)?;

        project.root_cid = Some(root_cid);
        project.updated_at = Utc::now();

        let json = serde_json::to_string_pretty(&project)?;
        fs::write(self.project_path(id), json)?;

        Ok(project)
    }

    fn list(&self) -> Result<Vec<String>, ProjectError> {
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

        let created = store.create(&pubkey).expect("create should succeed");

        assert_eq!(created.id.len(), 12, "project ID should be 12 characters");
        assert!(
            created.root_cid.is_none(),
            "new project should have no root CID"
        );
        assert_eq!(created.write_pubkey, pubkey);

        let fetched = store.get(&created.id).expect("get should succeed");

        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.root_cid, created.root_cid);
        assert_eq!(fetched.write_pubkey, created.write_pubkey);
        assert_eq!(fetched.created_at, created.created_at);
        assert_eq!(fetched.updated_at, created.updated_at);
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

        let created = store.create(&pubkey).expect("create should succeed");
        assert!(created.root_cid.is_none());

        let cid = Cid::from_bytes(b"project content v1");
        let updated = store
            .update_root(&created.id, cid.clone())
            .expect("update_root should succeed");

        assert_eq!(updated.root_cid.as_ref(), Some(&cid));
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
    }

    #[test]
    fn list_returns_created_projects() {
        let (store, _dir) = tmp_store();
        let pubkey = test_pubkey();

        let p1 = store.create(&pubkey).expect("create p1");
        let p2 = store.create(&pubkey).expect("create p2");
        let p3 = store.create(&pubkey).expect("create p3");

        let mut ids = store.list().expect("list should succeed");
        ids.sort();

        let mut expected = vec![p1.id, p2.id, p3.id];
        expected.sort();

        assert_eq!(ids, expected);
    }
}
