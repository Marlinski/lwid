//! Manifest types shared between client and server.
//!
//! A manifest describes one version (snapshot) of a project's file tree.
//! Manifests are stored as **plaintext** JSON blobs so the server can inspect
//! metadata (file paths, sizes, blob CIDs) while actual file contents remain
//! encrypted.

use serde::{Deserialize, Serialize};

/// A single file entry inside a manifest.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileEntry {
    /// Relative path within the project (e.g. `"src/index.html"`).
    pub path: String,
    /// CID of the encrypted blob that stores this file's content.
    pub cid: String,
    /// Original (plaintext) file size in bytes.
    pub size: u64,
}

/// A project manifest — a snapshot of the file tree at a specific version.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    /// 1-based version number.
    pub version: u64,
    /// CID of the parent (previous) manifest, or `None` for the first version.
    pub parent_cid: Option<String>,
    /// ISO 8601 timestamp of when this version was created.
    pub timestamp: String,
    /// The files included in this version.
    pub files: Vec<FileEntry>,
}

impl Manifest {
    /// Total plaintext size of all files in this manifest.
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// Collect all blob CIDs referenced by this manifest.
    pub fn blob_cids(&self) -> Vec<&str> {
        self.files.iter().map(|f| f.cid.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest() -> Manifest {
        Manifest {
            version: 1,
            parent_cid: None,
            timestamp: "2026-03-20T12:00:00Z".to_string(),
            files: vec![
                FileEntry {
                    path: "index.html".to_string(),
                    cid: "bafk1".to_string(),
                    size: 1024,
                },
                FileEntry {
                    path: "style.css".to_string(),
                    cid: "bafk2".to_string(),
                    size: 512,
                },
            ],
        }
    }

    #[test]
    fn total_size_sums_files() {
        let m = sample_manifest();
        assert_eq!(m.total_size(), 1536);
    }

    #[test]
    fn blob_cids_lists_all() {
        let m = sample_manifest();
        assert_eq!(m.blob_cids(), vec!["bafk1", "bafk2"]);
    }

    #[test]
    fn roundtrip_json() {
        let m = sample_manifest();
        let json = serde_json::to_string(&m).unwrap();
        let parsed: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, m.version);
        assert_eq!(parsed.files.len(), 2);
    }
}
