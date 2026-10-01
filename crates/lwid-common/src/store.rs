//! Content-addressed blob store.
//!
//! Blobs are stored and retrieved by their [`Cid`] — a deterministic,
//! content-derived identifier computed from the raw bytes (see [`crate::cid`]).
//!
//! The primary implementation, [`FsBlobStore`], persists blobs to the local
//! filesystem using the sharded directory layout produced by [`Cid::to_path`].

use std::fs;
use std::path::PathBuf;

use async_trait::async_trait;
use thiserror::Error;

use crate::cid::Cid;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by blob-store operations.
#[derive(Debug, Error)]
pub enum StoreError {
    /// An underlying I/O operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The requested CID does not exist in the store.
    #[error("blob not found: {cid}")]
    NotFound { cid: String },
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

/// Aggregate size of a blob store, as reported by [`BlobStore::usage`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoreUsage {
    /// Number of stored blobs.
    pub objects: u64,
    /// Total bytes occupied by those blobs.
    pub bytes: u64,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// A content-addressed blob store.
///
/// Implementations must be safe to share across threads.
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Store `data` and return its [`Cid`].
    ///
    /// The operation is **idempotent**: storing the same bytes twice must
    /// succeed and return the same CID without duplicating data.
    async fn put(&self, data: &[u8]) -> Result<Cid, StoreError>;

    /// Retrieve the raw bytes associated with `cid`.
    ///
    /// Returns [`StoreError::NotFound`] if no blob with that CID exists.
    async fn get(&self, cid: &Cid) -> Result<Vec<u8>, StoreError>;

    /// Check whether a blob with the given `cid` is present in the store.
    async fn exists(&self, cid: &Cid) -> Result<bool, StoreError>;

    /// Delete a blob by its [`Cid`].
    ///
    /// Returns [`StoreError::NotFound`] if the blob does not exist.
    async fn delete(&self, cid: &Cid) -> Result<(), StoreError>;

    /// Total object count and byte size of the store.
    ///
    /// This is a full enumeration — a directory walk for [`FsBlobStore`], a
    /// paginated `ListObjectsV2` for the S3 backend — so it costs O(objects)
    /// and issues real network calls. It exists for periodic reporting (the
    /// server's metrics exporter caches it) and must never be called from a
    /// request path.
    async fn usage(&self) -> Result<StoreUsage, StoreError>;
}

// ---------------------------------------------------------------------------
// Filesystem implementation
// ---------------------------------------------------------------------------

/// A [`BlobStore`] backed by the local filesystem.
///
/// Blobs are written into a sharded directory tree under `base_dir` using the
/// path layout defined by [`Cid::to_path`] (e.g. `ba/fk/<full-cid>`).
pub struct FsBlobStore {
    base_dir: PathBuf,
}

impl FsBlobStore {
    /// Create a new filesystem blob store rooted at `base_dir`.
    ///
    /// The directory (and any missing parents) is created if it does not
    /// already exist.
    pub fn new(base_dir: PathBuf) -> Result<Self, StoreError> {
        fs::create_dir_all(&base_dir)?;
        Ok(Self { base_dir })
    }

    /// Resolve a [`Cid`] to its absolute filesystem path.
    fn blob_path(&self, cid: &Cid) -> PathBuf {
        self.base_dir.join(cid.to_path())
    }
}

#[async_trait]
impl BlobStore for FsBlobStore {
    async fn put(&self, data: &[u8]) -> Result<Cid, StoreError> {
        let cid = Cid::from_bytes(data);
        let path = self.blob_path(&cid);

        // Idempotent: skip the write if the file already exists.
        if path.exists() {
            return Ok(cid);
        }

        // Ensure the parent shard directories exist.
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&path, data)?;
        Ok(cid)
    }

    async fn get(&self, cid: &Cid) -> Result<Vec<u8>, StoreError> {
        let path = self.blob_path(cid);

        if !path.exists() {
            return Err(StoreError::NotFound {
                cid: cid.to_string(),
            });
        }

        Ok(fs::read(&path)?)
    }

    async fn exists(&self, cid: &Cid) -> Result<bool, StoreError> {
        let path = self.blob_path(cid);
        Ok(path.exists())
    }

    async fn delete(&self, cid: &Cid) -> Result<(), StoreError> {
        let path = self.blob_path(cid);

        if !path.exists() {
            return Err(StoreError::NotFound {
                cid: cid.to_string(),
            });
        }

        fs::remove_file(&path)?;
        Ok(())
    }

    async fn usage(&self) -> Result<StoreUsage, StoreError> {
        fn walk(dir: &std::path::Path, acc: &mut StoreUsage) -> Result<(), std::io::Error> {
            let entries = match fs::read_dir(dir) {
                Ok(entries) => entries,
                // A store that has never been written to may have no tree at
                // all. That is an empty store, not a failure.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e),
            };
            for entry in entries {
                let entry = entry?;
                let meta = entry.metadata()?;
                if meta.is_dir() {
                    walk(&entry.path(), acc)?;
                } else {
                    acc.objects += 1;
                    acc.bytes += meta.len();
                }
            }
            Ok(())
        }

        let mut usage = StoreUsage::default();
        walk(&self.base_dir, &mut usage)?;
        Ok(usage)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create a temporary [`FsBlobStore`].
    fn tmp_store() -> (FsBlobStore, TempDir) {
        let dir = TempDir::new().expect("failed to create temp dir");
        let store =
            FsBlobStore::new(dir.path().join("blobs")).expect("failed to create FsBlobStore");
        (store, dir)
    }

    #[tokio::test]
    async fn usage_counts_objects_and_bytes() {
        let (store, _dir) = tmp_store();
        assert_eq!(store.usage().await.unwrap(), StoreUsage::default());

        store.put(b"hello").await.unwrap();
        store.put(b"worldly").await.unwrap();
        let usage = store.usage().await.unwrap();
        assert_eq!(usage.objects, 2);
        assert_eq!(usage.bytes, 5 + 7);

        // Content addressing means a duplicate put adds nothing.
        store.put(b"hello").await.unwrap();
        assert_eq!(store.usage().await.unwrap(), usage);
    }

    #[tokio::test]
    async fn usage_of_absent_tree_is_empty_not_an_error() {
        let dir = TempDir::new().expect("temp dir");
        let store = FsBlobStore::new(dir.path().join("blobs")).expect("store");
        std::fs::remove_dir_all(dir.path().join("blobs")).expect("remove tree");
        assert_eq!(store.usage().await.unwrap(), StoreUsage::default());
    }

    #[tokio::test]
    async fn put_get_roundtrip() {
        let (store, _dir) = tmp_store();
        let data = b"hello, content-addressed world!";

        let cid = store.put(data).await.expect("put should succeed");
        let retrieved = store.get(&cid).await.expect("get should succeed");

        assert_eq!(retrieved, data, "retrieved bytes must match original data");
    }

    #[tokio::test]
    async fn idempotent_put() {
        let (store, _dir) = tmp_store();
        let data = b"idempotent payload";

        let cid1 = store.put(data).await.expect("first put");
        let cid2 = store.put(data).await.expect("second put");

        assert_eq!(
            cid1, cid2,
            "putting the same data twice must return the same CID"
        );

        // The file should still contain the original data.
        let retrieved = store.get(&cid1).await.expect("get after double put");
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn get_nonexistent_returns_not_found() {
        let (store, _dir) = tmp_store();
        let cid = Cid::from_bytes(b"data that was never stored");

        let err = store
            .get(&cid)
            .await
            .expect_err("get should fail for missing CID");

        match err {
            StoreError::NotFound { cid: ref s } => {
                assert_eq!(s, cid.as_str());
            }
            _ => panic!("expected StoreError::NotFound, got: {err:?}"),
        }
    }

    #[tokio::test]
    async fn exists_returns_true_after_put() {
        let (store, _dir) = tmp_store();
        let data = b"existence check";

        let cid = store.put(data).await.expect("put should succeed");

        assert!(
            store.exists(&cid).await.expect("exists should succeed"),
            "exists must return true for a stored blob",
        );
    }

    #[tokio::test]
    async fn exists_returns_false_for_missing() {
        let (store, _dir) = tmp_store();
        let cid = Cid::from_bytes(b"never stored");

        assert!(
            !store.exists(&cid).await.expect("exists should succeed"),
            "exists must return false for a missing blob",
        );
    }
}
