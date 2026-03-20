//! Mutable key-value store for per-project data.
//!
//! Unlike the content-addressed [`BlobStore`](crate::store::BlobStore), the KV
//! store allows in-place overwrites — a simple PUT replaces the previous value.
//! Each project gets its own flat namespace of keys.

use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::limits::{MAX_STORE_KEY_LENGTH, MAX_STORE_TOTAL_SIZE, MAX_STORE_VALUE_SIZE};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by key-value store operations.
#[derive(Debug, Error)]
pub enum KvError {
    /// An underlying I/O operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The requested key does not exist.
    #[error("key not found: project={project_id}, key={key}")]
    NotFound { project_id: String, key: String },

    /// The key is syntactically invalid.
    #[error("invalid key \"{key}\": {reason}")]
    InvalidKey { key: String, reason: String },

    /// Writing the value would exceed the per-project quota.
    #[error("quota exceeded for project {project_id}: limit={limit}, current={current}")]
    QuotaExceeded {
        project_id: String,
        limit: u64,
        current: u64,
    },
}

// ---------------------------------------------------------------------------
// Key validation
// ---------------------------------------------------------------------------

/// Validate a store key.
///
/// Allowed characters: alphanumeric, `-`, `_`, `:`, `.`, `/`.
/// Max length: [`MAX_STORE_KEY_LENGTH`] characters.
/// No `..` segments (path traversal prevention).
/// Keys starting with `__` are reserved.
fn validate_key(key: &str) -> Result<(), KvError> {
    if key.is_empty() {
        return Err(KvError::InvalidKey {
            key: key.to_owned(),
            reason: "key must not be empty".to_owned(),
        });
    }

    if key.len() > MAX_STORE_KEY_LENGTH {
        return Err(KvError::InvalidKey {
            key: key.to_owned(),
            reason: format!("key exceeds maximum length of {MAX_STORE_KEY_LENGTH} characters"),
        });
    }

    if key.starts_with("__") {
        return Err(KvError::InvalidKey {
            key: key.to_owned(),
            reason: "keys starting with \"__\" are reserved".to_owned(),
        });
    }

    if key.contains("..") {
        return Err(KvError::InvalidKey {
            key: key.to_owned(),
            reason: "key must not contain \"..\" (path traversal)".to_owned(),
        });
    }

    // Disallow leading/trailing slashes and empty segments
    if key.starts_with('/') || key.ends_with('/') {
        return Err(KvError::InvalidKey {
            key: key.to_owned(),
            reason: "key must not start or end with \"/\"".to_owned(),
        });
    }

    if key.contains("//") {
        return Err(KvError::InvalidKey {
            key: key.to_owned(),
            reason: "key must not contain empty path segments (\"//\")".to_owned(),
        });
    }

    for ch in key.chars() {
        if !(ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == ':' || ch == '.' || ch == '/')
        {
            return Err(KvError::InvalidKey {
                key: key.to_owned(),
                reason: format!(
                    "invalid character '{ch}': allowed are alphanumeric, -, _, :, ., /"
                ),
            });
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// A mutable key-value store scoped per project.
///
/// Implementations must be safe to share across threads.
pub trait KvStore: Send + Sync {
    /// Write a value for the given key, overwriting any previous value.
    fn put(&self, project_id: &str, key: &str, value: &[u8]) -> Result<(), KvError>;

    /// Read the value for the given key.
    fn get(&self, project_id: &str, key: &str) -> Result<Vec<u8>, KvError>;

    /// Delete a key and its value.
    fn delete(&self, project_id: &str, key: &str) -> Result<(), KvError>;

    /// List all keys for a project.
    fn list_keys(&self, project_id: &str) -> Result<Vec<String>, KvError>;

    /// List all keys with their value sizes (in bytes) for a project.
    fn list_keys_with_sizes(&self, project_id: &str) -> Result<Vec<(String, u64)>, KvError>;

    /// Delete all keys for a project (used by the reaper on project expiry).
    fn delete_all(&self, project_id: &str) -> Result<(), KvError>;

    /// Compute the total size of all stored values for a project, in bytes.
    fn total_size(&self, project_id: &str) -> Result<u64, KvError>;
}

// ---------------------------------------------------------------------------
// Filesystem implementation
// ---------------------------------------------------------------------------

/// A [`KvStore`] backed by the local filesystem.
///
/// Layout: `{base_dir}/{project_id}/{key}` — slashes in keys map to
/// subdirectories on disk.
pub struct FsKvStore {
    base_dir: PathBuf,
}

impl FsKvStore {
    /// Create a new filesystem KV store rooted at `base_dir`.
    ///
    /// The directory (and any missing parents) is created if it does not
    /// already exist.
    pub fn new(base_dir: PathBuf) -> Result<Self, KvError> {
        fs::create_dir_all(&base_dir)?;
        Ok(Self { base_dir })
    }

    /// Return the on-disk path for a given project + key.
    fn key_path(&self, project_id: &str, key: &str) -> PathBuf {
        self.base_dir.join(project_id).join(key)
    }

    /// Return the project directory.
    fn project_dir(&self, project_id: &str) -> PathBuf {
        self.base_dir.join(project_id)
    }

    /// Recursively walk a directory and collect all file paths relative to `root`.
    fn walk_keys(dir: &Path, root: &Path, out: &mut Vec<String>) -> Result<(), KvError> {
        if !dir.exists() {
            return Ok(());
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                Self::walk_keys(&path, root, out)?;
            } else if let Ok(relative) = path.strip_prefix(root) {
                if let Some(key) = relative.to_str() {
                    out.push(key.to_owned());
                }
            }
        }

        Ok(())
    }

    /// Recursively walk a directory and collect all file paths with sizes relative to `root`.
    fn walk_keys_with_sizes(
        dir: &Path,
        root: &Path,
        out: &mut Vec<(String, u64)>,
    ) -> Result<(), KvError> {
        if !dir.exists() {
            return Ok(());
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                Self::walk_keys_with_sizes(&path, root, out)?;
            } else if let Ok(relative) = path.strip_prefix(root) {
                if let Some(key) = relative.to_str() {
                    let size = entry.metadata()?.len();
                    out.push((key.to_owned(), size));
                }
            }
        }

        Ok(())
    }

    /// Recursively compute total size of all files under a directory.
    fn walk_size(dir: &Path) -> Result<u64, KvError> {
        if !dir.exists() {
            return Ok(0);
        }

        let mut total = 0u64;
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                total += Self::walk_size(&path)?;
            } else {
                total += entry.metadata()?.len();
            }
        }

        Ok(total)
    }
}

impl KvStore for FsKvStore {
    fn put(&self, project_id: &str, key: &str, value: &[u8]) -> Result<(), KvError> {
        validate_key(key)?;

        if value.len() > MAX_STORE_VALUE_SIZE {
            return Err(KvError::QuotaExceeded {
                project_id: project_id.to_owned(),
                limit: MAX_STORE_VALUE_SIZE as u64,
                current: value.len() as u64,
            });
        }

        // Check total size quota. We need to account for the existing value
        // (if any) being replaced.
        let project_dir = self.project_dir(project_id);
        let current_total = Self::walk_size(&project_dir)?;
        let key_path = self.key_path(project_id, key);
        let existing_size = if key_path.exists() {
            fs::metadata(&key_path)?.len()
        } else {
            0
        };
        let new_total = current_total - existing_size + value.len() as u64;
        if new_total > MAX_STORE_TOTAL_SIZE as u64 {
            return Err(KvError::QuotaExceeded {
                project_id: project_id.to_owned(),
                limit: MAX_STORE_TOTAL_SIZE as u64,
                current: current_total,
            });
        }

        // Ensure parent directories exist (for path-like keys).
        if let Some(parent) = key_path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&key_path, value)?;
        Ok(())
    }

    fn get(&self, project_id: &str, key: &str) -> Result<Vec<u8>, KvError> {
        validate_key(key)?;

        let path = self.key_path(project_id, key);
        if !path.exists() {
            return Err(KvError::NotFound {
                project_id: project_id.to_owned(),
                key: key.to_owned(),
            });
        }

        Ok(fs::read(&path)?)
    }

    fn delete(&self, project_id: &str, key: &str) -> Result<(), KvError> {
        validate_key(key)?;

        let path = self.key_path(project_id, key);
        if !path.exists() {
            return Err(KvError::NotFound {
                project_id: project_id.to_owned(),
                key: key.to_owned(),
            });
        }

        fs::remove_file(&path)?;

        // Clean up empty parent directories (but not the project dir itself).
        let project_dir = self.project_dir(project_id);
        let mut current = path.parent().map(Path::to_path_buf);
        while let Some(ref dir) = current {
            if dir == &project_dir {
                break;
            }
            // Try to remove — will fail (harmlessly) if not empty.
            if fs::remove_dir(dir).is_err() {
                break;
            }
            current = dir.parent().map(Path::to_path_buf);
        }

        Ok(())
    }

    fn list_keys(&self, project_id: &str) -> Result<Vec<String>, KvError> {
        let project_dir = self.project_dir(project_id);
        let mut keys = Vec::new();
        Self::walk_keys(&project_dir, &project_dir, &mut keys)?;
        keys.sort();
        Ok(keys)
    }

    fn list_keys_with_sizes(&self, project_id: &str) -> Result<Vec<(String, u64)>, KvError> {
        let project_dir = self.project_dir(project_id);
        let mut entries = Vec::new();
        Self::walk_keys_with_sizes(&project_dir, &project_dir, &mut entries)?;
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(entries)
    }

    fn delete_all(&self, project_id: &str) -> Result<(), KvError> {
        let project_dir = self.project_dir(project_id);
        if project_dir.exists() {
            fs::remove_dir_all(&project_dir)?;
        }
        Ok(())
    }

    fn total_size(&self, project_id: &str) -> Result<u64, KvError> {
        let project_dir = self.project_dir(project_id);
        Self::walk_size(&project_dir)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create a temporary [`FsKvStore`].
    fn tmp_store() -> (FsKvStore, TempDir) {
        let dir = TempDir::new().expect("failed to create temp dir");
        let store = FsKvStore::new(dir.path().join("kv")).expect("failed to create FsKvStore");
        (store, dir)
    }

    #[test]
    fn put_get_roundtrip() {
        let (store, _dir) = tmp_store();
        let data = b"hello kv world";

        store
            .put("proj1", "greeting", data)
            .expect("put should succeed");
        let retrieved = store.get("proj1", "greeting").expect("get should succeed");

        assert_eq!(retrieved, data, "retrieved bytes must match original data");
    }

    #[test]
    fn put_overwrites_existing() {
        let (store, _dir) = tmp_store();

        store.put("proj1", "key", b"v1").expect("put v1");
        store.put("proj1", "key", b"v2").expect("put v2");

        let retrieved = store.get("proj1", "key").expect("get after overwrite");
        assert_eq!(retrieved, b"v2");
    }

    #[test]
    fn get_nonexistent_returns_not_found() {
        let (store, _dir) = tmp_store();

        let err = store
            .get("proj1", "missing")
            .expect_err("get should fail for missing key");

        match err {
            KvError::NotFound {
                ref project_id,
                ref key,
            } => {
                assert_eq!(project_id, "proj1");
                assert_eq!(key, "missing");
            }
            _ => panic!("expected KvError::NotFound, got: {err:?}"),
        }
    }

    #[test]
    fn delete_removes_key() {
        let (store, _dir) = tmp_store();

        store.put("proj1", "key", b"data").expect("put");
        store.delete("proj1", "key").expect("delete should succeed");

        let err = store
            .get("proj1", "key")
            .expect_err("get after delete should fail");
        assert!(matches!(err, KvError::NotFound { .. }));
    }

    #[test]
    fn delete_nonexistent_returns_not_found() {
        let (store, _dir) = tmp_store();

        let err = store
            .delete("proj1", "missing")
            .expect_err("delete should fail for missing key");
        assert!(matches!(err, KvError::NotFound { .. }));
    }

    #[test]
    fn list_keys_returns_sorted() {
        let (store, _dir) = tmp_store();

        store.put("proj1", "c", b"3").expect("put c");
        store.put("proj1", "a", b"1").expect("put a");
        store.put("proj1", "b", b"2").expect("put b");

        let keys = store.list_keys("proj1").expect("list_keys");
        assert_eq!(keys, vec!["a", "b", "c"]);
    }

    #[test]
    fn list_keys_empty_project() {
        let (store, _dir) = tmp_store();

        let keys = store
            .list_keys("proj-empty")
            .expect("list_keys on empty project");
        assert!(keys.is_empty());
    }

    #[test]
    fn delete_all_removes_everything() {
        let (store, _dir) = tmp_store();

        store.put("proj1", "a", b"1").expect("put a");
        store.put("proj1", "b", b"2").expect("put b");

        store.delete_all("proj1").expect("delete_all");

        let keys = store
            .list_keys("proj1")
            .expect("list_keys after delete_all");
        assert!(keys.is_empty());
    }

    #[test]
    fn delete_all_nonexistent_project_is_ok() {
        let (store, _dir) = tmp_store();
        store
            .delete_all("nonexistent")
            .expect("delete_all on missing project should be ok");
    }

    #[test]
    fn total_size_computes_correctly() {
        let (store, _dir) = tmp_store();

        store.put("proj1", "a", b"12345").expect("put a");
        store.put("proj1", "b", b"67890").expect("put b");

        let size = store.total_size("proj1").expect("total_size");
        assert_eq!(size, 10);
    }

    #[test]
    fn total_size_empty_project() {
        let (store, _dir) = tmp_store();
        let size = store.total_size("proj-empty").expect("total_size on empty");
        assert_eq!(size, 0);
    }

    #[test]
    fn path_like_keys_work() {
        let (store, _dir) = tmp_store();

        store
            .put("proj1", "uploads/photo.png", b"png-data")
            .expect("put path-like key");

        let retrieved = store
            .get("proj1", "uploads/photo.png")
            .expect("get path-like key");
        assert_eq!(retrieved, b"png-data");

        let keys = store.list_keys("proj1").expect("list_keys");
        assert_eq!(keys, vec!["uploads/photo.png"]);

        store
            .delete("proj1", "uploads/photo.png")
            .expect("delete path-like key");

        let keys = store.list_keys("proj1").expect("list_keys after delete");
        assert!(keys.is_empty());
    }

    #[test]
    fn nested_path_keys() {
        let (store, _dir) = tmp_store();

        store
            .put("proj1", "a/b/c.txt", b"deep")
            .expect("put nested");
        store
            .put("proj1", "a/d.txt", b"shallow")
            .expect("put shallow");

        let keys = store.list_keys("proj1").expect("list_keys");
        assert_eq!(keys, vec!["a/b/c.txt", "a/d.txt"]);

        let size = store.total_size("proj1").expect("total_size");
        assert_eq!(size, 4 + 7); // "deep" + "shallow"
    }

    #[test]
    fn invalid_key_empty() {
        let (store, _dir) = tmp_store();
        let err = store
            .put("proj1", "", b"data")
            .expect_err("empty key should fail");
        assert!(matches!(err, KvError::InvalidKey { .. }));
    }

    #[test]
    fn invalid_key_path_traversal() {
        let (store, _dir) = tmp_store();
        let err = store
            .put("proj1", "../escape", b"data")
            .expect_err(".. should be rejected");
        assert!(matches!(err, KvError::InvalidKey { .. }));

        let err = store
            .put("proj1", "a/../b", b"data")
            .expect_err(".. in path should be rejected");
        assert!(matches!(err, KvError::InvalidKey { .. }));
    }

    #[test]
    fn invalid_key_reserved_prefix() {
        let (store, _dir) = tmp_store();
        let err = store
            .put("proj1", "__internal", b"data")
            .expect_err("__ prefix should be rejected");
        assert!(matches!(err, KvError::InvalidKey { .. }));
    }

    #[test]
    fn invalid_key_bad_characters() {
        let (store, _dir) = tmp_store();
        let err = store
            .put("proj1", "key with spaces", b"data")
            .expect_err("spaces should be rejected");
        assert!(matches!(err, KvError::InvalidKey { .. }));
    }

    #[test]
    fn invalid_key_leading_trailing_slash() {
        let (store, _dir) = tmp_store();

        let err = store
            .put("proj1", "/leading", b"data")
            .expect_err("leading slash should be rejected");
        assert!(matches!(err, KvError::InvalidKey { .. }));

        let err = store
            .put("proj1", "trailing/", b"data")
            .expect_err("trailing slash should be rejected");
        assert!(matches!(err, KvError::InvalidKey { .. }));
    }

    #[test]
    fn projects_are_isolated() {
        let (store, _dir) = tmp_store();

        store.put("proj1", "key", b"from-proj1").expect("put proj1");
        store.put("proj2", "key", b"from-proj2").expect("put proj2");

        assert_eq!(store.get("proj1", "key").expect("get proj1"), b"from-proj1");
        assert_eq!(store.get("proj2", "key").expect("get proj2"), b"from-proj2");

        store.delete_all("proj1").expect("delete_all proj1");

        // proj2 should be unaffected.
        assert_eq!(
            store
                .get("proj2", "key")
                .expect("get proj2 after proj1 deleted"),
            b"from-proj2"
        );
    }
}
