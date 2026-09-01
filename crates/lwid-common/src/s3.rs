//! S3-backed implementations of the three storage traits.
//!
//! These mirror the filesystem stores ([`crate::store::FsBlobStore`],
//! [`crate::project::FsProjectStore`], [`crate::kv::FsKvStore`]) exactly,
//! talking directly to an S3-compatible object store instead of a mounted
//! filesystem. No relational store, no metadata service, no persistent volume.
//!
//! # Key layout
//!
//! The bucket layout is byte-for-byte the same as the on-disk layout, so a
//! filesystem data directory can be copied into a bucket verbatim (and back):
//!
//! ```text
//! {prefix}blobs/ab/cd/<full-cid>      content-addressed blobs (immutable)
//! {prefix}projects/<project-id>.json  project metadata
//! {prefix}store/<project-id>/<key>    per-project KV / blob store
//! ```
//!
//! # Consistency
//!
//! Blobs are immutable and content-addressed, so they need no coordination.
//! Project metadata is read-modify-write ([`ProjectStore::update_root`],
//! [`ProjectStore::update_expiry`]); concurrent writers on *different* server
//! replicas can therefore lose an update. This matches the existing
//! filesystem behaviour and is safe for a single replica.

use std::collections::BTreeSet;

use async_trait::async_trait;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::operation::head_object::HeadObjectError;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{Delete, ObjectIdentifier};
use aws_sdk_s3::Client;
use chrono::{DateTime, Utc};

use crate::cid::Cid;
use crate::kv::{validate_key, KvError, KvStore};
use crate::limits::{MAX_STORE_TOTAL_SIZE, MAX_STORE_VALUE_SIZE};
use crate::project::{Project, ProjectError, ProjectStore};
use crate::store::{BlobStore, StoreError};

/// Maximum number of keys S3 accepts in a single `DeleteObjects` request.
const DELETE_BATCH_SIZE: usize = 1000;

// ---------------------------------------------------------------------------
// Client construction
// ---------------------------------------------------------------------------

/// Connection settings for an S3-compatible endpoint.
#[derive(Clone, Debug)]
pub struct S3Settings {
    /// Endpoint URL, e.g. `https://s3.gra.io.cloud.ovh.net`.
    pub endpoint: String,
    /// Region name, e.g. `gra`.
    pub region: String,
    /// Bucket that holds all lwid data.
    pub bucket: String,
    /// Optional key prefix within the bucket. Normalised to end with `/`.
    pub prefix: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    /// Use path-style addressing (`endpoint/bucket/key`) rather than
    /// virtual-host style (`bucket.endpoint/key`).
    pub force_path_style: bool,
}

impl S3Settings {
    /// Normalise the prefix so it is either empty or ends with exactly one `/`.
    pub fn normalized_prefix(&self) -> String {
        let trimmed = self.prefix.trim_matches('/');
        if trimmed.is_empty() {
            String::new()
        } else {
            format!("{trimmed}/")
        }
    }
}

/// Build an S3 client from [`S3Settings`].
pub fn client(settings: &S3Settings) -> Client {
    let creds = Credentials::new(
        &settings.access_key_id,
        &settings.secret_access_key,
        None,
        None,
        "lwid-config",
    );

    let conf = aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(settings.region.clone()))
        .endpoint_url(&settings.endpoint)
        .credentials_provider(creds)
        .force_path_style(settings.force_path_style)
        .build();

    Client::from_conf(conf)
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

/// Wrap any S3 failure as an I/O error.
///
/// Every one of the three error enums already models transport failure as
/// `Io`, and the HTTP layer maps that to `500`, which is the right answer for
/// a backend that is unreachable, misconfigured, or refusing us.
fn io_err(context: &str, e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(format!("s3: {context}: {e}"))
}

/// True if a `GetObject` failure means "no such key".
fn get_is_missing<R>(err: &SdkError<GetObjectError, R>) -> bool {
    matches!(err, SdkError::ServiceError(e) if e.err().is_no_such_key())
}

/// True if a `HeadObject` failure means "no such key".
///
/// `HeadObject` cannot return a body, so S3 reports a missing key as a bare
/// 404 rather than the `NoSuchKey` error code used by `GetObject`.
fn head_is_missing<R>(err: &SdkError<HeadObjectError, R>) -> bool {
    matches!(err, SdkError::ServiceError(e) if e.err().is_not_found())
}

// ---------------------------------------------------------------------------
// Shared object helpers
// ---------------------------------------------------------------------------

/// A single listed object: its key relative to the listing prefix, and size.
struct Listed {
    relative_key: String,
    size: u64,
}

/// List every object under `prefix`, following continuation tokens.
///
/// Returns keys *relative* to `prefix`. Objects whose key ends in `/`
/// (directory placeholders some tools create) are skipped.
async fn list_all(client: &Client, bucket: &str, prefix: &str) -> Result<Vec<Listed>, String> {
    let mut out = Vec::new();
    let mut continuation: Option<String> = None;

    loop {
        let mut req = client.list_objects_v2().bucket(bucket).prefix(prefix);
        if let Some(token) = continuation.take() {
            req = req.continuation_token(token);
        }

        let resp = req.send().await.map_err(|e| e.to_string())?;

        for obj in resp.contents() {
            let Some(key) = obj.key() else { continue };
            if key.ends_with('/') {
                continue;
            }
            let Some(relative) = key.strip_prefix(prefix) else {
                continue;
            };
            out.push(Listed {
                relative_key: relative.to_owned(),
                size: obj.size().unwrap_or(0).max(0) as u64,
            });
        }

        if resp.is_truncated().unwrap_or(false) {
            continuation = resp.next_continuation_token().map(str::to_owned);
            if continuation.is_none() {
                break;
            }
        } else {
            break;
        }
    }

    Ok(out)
}

/// Delete every object under `prefix`, in batches.
async fn delete_all_under(client: &Client, bucket: &str, prefix: &str) -> Result<(), String> {
    let listed = list_all(client, bucket, prefix).await?;

    for chunk in listed.chunks(DELETE_BATCH_SIZE) {
        let mut delete = Delete::builder().quiet(true);
        for item in chunk {
            let id = ObjectIdentifier::builder()
                .key(format!("{prefix}{}", item.relative_key))
                .build()
                .map_err(|e| e.to_string())?;
            delete = delete.objects(id);
        }
        let delete = delete.build().map_err(|e| e.to_string())?;

        client
            .delete_objects()
            .bucket(bucket)
            .delete(delete)
            .send()
            .await
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Blob store
// ---------------------------------------------------------------------------

/// A [`BlobStore`] backed by an S3-compatible bucket.
pub struct S3BlobStore {
    client: Client,
    bucket: String,
    prefix: String,
}

impl S3BlobStore {
    pub fn new(client: Client, bucket: String, prefix: String) -> Self {
        Self {
            client,
            bucket,
            prefix,
        }
    }

    fn key(&self, cid: &Cid) -> String {
        format!("{}blobs/{}", self.prefix, cid.to_key())
    }
}

#[async_trait]
impl BlobStore for S3BlobStore {
    async fn put(&self, data: &[u8]) -> Result<Cid, StoreError> {
        let cid = Cid::from_bytes(data);
        let key = self.key(&cid);

        // Idempotent: the key is derived from the content, so if the object is
        // already there its bytes are identical and re-uploading is waste.
        if self.exists(&cid).await? {
            return Ok(cid);
        }

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(ByteStream::from(data.to_vec()))
            .send()
            .await
            .map_err(|e| io_err(&format!("put_object {key}"), e))?;

        Ok(cid)
    }

    async fn get(&self, cid: &Cid) -> Result<Vec<u8>, StoreError> {
        let key = self.key(cid);

        let resp = match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) if get_is_missing(&e) => {
                return Err(StoreError::NotFound {
                    cid: cid.to_string(),
                })
            }
            Err(e) => return Err(io_err(&format!("get_object {key}"), e).into()),
        };

        let bytes = resp
            .body
            .collect()
            .await
            .map_err(|e| io_err(&format!("read body {key}"), e))?;

        Ok(bytes.into_bytes().to_vec())
    }

    async fn exists(&self, cid: &Cid) -> Result<bool, StoreError> {
        let key = self.key(cid);

        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) if head_is_missing(&e) => Ok(false),
            Err(e) => Err(io_err(&format!("head_object {key}"), e).into()),
        }
    }

    async fn delete(&self, cid: &Cid) -> Result<(), StoreError> {
        // S3 `DeleteObject` succeeds on a missing key; the trait promises
        // `NotFound`, so check first.
        if !self.exists(cid).await? {
            return Err(StoreError::NotFound {
                cid: cid.to_string(),
            });
        }

        let key = self.key(cid);
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| io_err(&format!("delete_object {key}"), e))?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Project store
// ---------------------------------------------------------------------------

/// A [`ProjectStore`] backed by an S3-compatible bucket.
pub struct S3ProjectStore {
    client: Client,
    bucket: String,
    prefix: String,
}

impl S3ProjectStore {
    pub fn new(client: Client, bucket: String, prefix: String) -> Self {
        Self {
            client,
            bucket,
            prefix,
        }
    }

    fn key(&self, id: &str) -> String {
        format!("{}projects/{id}.json", self.prefix)
    }

    fn list_prefix(&self) -> String {
        format!("{}projects/", self.prefix)
    }

    /// Serialize and upload a project, overwriting any previous version.
    async fn write(&self, project: &Project) -> Result<(), ProjectError> {
        let json = serde_json::to_string_pretty(project)?;
        let key = self.key(&project.id);

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(ByteStream::from(json.into_bytes()))
            .content_type("application/json")
            .send()
            .await
            .map_err(|e| io_err(&format!("put_object {key}"), e))?;

        Ok(())
    }
}

#[async_trait]
impl ProjectStore for S3ProjectStore {
    async fn create(
        &self,
        write_pubkey: &[u8],
        expires_at: Option<DateTime<Utc>>,
        store_token: Option<String>,
        created_with: Option<String>,
    ) -> Result<Project, ProjectError> {
        let now = Utc::now();
        let project = Project {
            id: nanoid::nanoid!(12),
            root_cid: None,
            write_pubkey: write_pubkey.to_vec(),
            created_at: now,
            updated_at: now,
            expires_at,
            blob_cids: BTreeSet::new(),
            store_token,
            created_with,
        };

        self.write(&project).await?;
        Ok(project)
    }

    async fn get(&self, id: &str) -> Result<Project, ProjectError> {
        let key = self.key(id);

        let resp = match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) if get_is_missing(&e) => {
                return Err(ProjectError::NotFound { id: id.to_owned() })
            }
            Err(e) => return Err(io_err(&format!("get_object {key}"), e).into()),
        };

        let bytes = resp
            .body
            .collect()
            .await
            .map_err(|e| io_err(&format!("read body {key}"), e))?;

        let project: Project = serde_json::from_slice(&bytes.into_bytes())?;
        Ok(project)
    }

    async fn update_root(
        &self,
        id: &str,
        root_cid: Cid,
        blob_cids: BTreeSet<String>,
    ) -> Result<Project, ProjectError> {
        let mut project = self.get(id).await?;

        project.root_cid = Some(root_cid);
        project.blob_cids = blob_cids;
        project.updated_at = Utc::now();

        self.write(&project).await?;
        Ok(project)
    }

    async fn delete(&self, id: &str) -> Result<Project, ProjectError> {
        let project = self.get(id).await?;
        let key = self.key(id);

        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| io_err(&format!("delete_object {key}"), e))?;

        Ok(project)
    }

    async fn update_expiry(
        &self,
        id: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<Project, ProjectError> {
        let mut project = self.get(id).await?;

        project.expires_at = expires_at;
        project.updated_at = Utc::now();

        self.write(&project).await?;
        Ok(project)
    }

    async fn list(&self) -> Result<Vec<String>, ProjectError> {
        let prefix = self.list_prefix();
        let listed = list_all(&self.client, &self.bucket, &prefix)
            .await
            .map_err(|e| io_err(&format!("list_objects {prefix}"), e))?;

        Ok(listed
            .into_iter()
            .filter_map(|o| {
                o.relative_key
                    .strip_suffix(".json")
                    .map(|stem| stem.to_owned())
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// KV store
// ---------------------------------------------------------------------------

/// A [`KvStore`] backed by an S3-compatible bucket.
pub struct S3KvStore {
    client: Client,
    bucket: String,
    prefix: String,
}

impl S3KvStore {
    pub fn new(client: Client, bucket: String, prefix: String) -> Self {
        Self {
            client,
            bucket,
            prefix,
        }
    }

    fn key(&self, project_id: &str, key: &str) -> String {
        format!("{}store/{project_id}/{key}", self.prefix)
    }

    fn project_prefix(&self, project_id: &str) -> String {
        format!("{}store/{project_id}/", self.prefix)
    }

    /// One listing pass over a project's keys, with sizes.
    async fn list_entries(&self, project_id: &str) -> Result<Vec<Listed>, KvError> {
        let prefix = self.project_prefix(project_id);
        list_all(&self.client, &self.bucket, &prefix)
            .await
            .map_err(|e| io_err(&format!("list_objects {prefix}"), e).into())
    }
}

#[async_trait]
impl KvStore for S3KvStore {
    async fn put(&self, project_id: &str, key: &str, value: &[u8]) -> Result<(), KvError> {
        validate_key(key)?;

        if value.len() > MAX_STORE_VALUE_SIZE {
            return Err(KvError::QuotaExceeded {
                project_id: project_id.to_owned(),
                limit: MAX_STORE_VALUE_SIZE as u64,
                current: value.len() as u64,
            });
        }

        // A single listing answers both questions the quota check needs: the
        // project's current total, and the size of the value being replaced.
        let entries = self.list_entries(project_id).await?;
        let current_total: u64 = entries.iter().map(|e| e.size).sum();
        let existing_size = entries
            .iter()
            .find(|e| e.relative_key == key)
            .map_or(0, |e| e.size);

        let new_total = current_total - existing_size + value.len() as u64;
        if new_total > MAX_STORE_TOTAL_SIZE as u64 {
            return Err(KvError::QuotaExceeded {
                project_id: project_id.to_owned(),
                limit: MAX_STORE_TOTAL_SIZE as u64,
                current: current_total,
            });
        }

        let object_key = self.key(project_id, key);
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&object_key)
            .body(ByteStream::from(value.to_vec()))
            .send()
            .await
            .map_err(|e| io_err(&format!("put_object {object_key}"), e))?;

        Ok(())
    }

    async fn get(&self, project_id: &str, key: &str) -> Result<Vec<u8>, KvError> {
        validate_key(key)?;
        let object_key = self.key(project_id, key);

        let resp = match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&object_key)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) if get_is_missing(&e) => {
                return Err(KvError::NotFound {
                    project_id: project_id.to_owned(),
                    key: key.to_owned(),
                })
            }
            Err(e) => return Err(io_err(&format!("get_object {object_key}"), e).into()),
        };

        let bytes = resp
            .body
            .collect()
            .await
            .map_err(|e| io_err(&format!("read body {object_key}"), e))?;

        Ok(bytes.into_bytes().to_vec())
    }

    async fn delete(&self, project_id: &str, key: &str) -> Result<(), KvError> {
        validate_key(key)?;
        let object_key = self.key(project_id, key);

        // S3 `DeleteObject` is a no-op on a missing key; the trait promises
        // `NotFound`, so probe first.
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(&object_key)
            .send()
            .await
        {
            Ok(_) => {}
            Err(e) if head_is_missing(&e) => {
                return Err(KvError::NotFound {
                    project_id: project_id.to_owned(),
                    key: key.to_owned(),
                })
            }
            Err(e) => return Err(io_err(&format!("head_object {object_key}"), e).into()),
        }

        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(&object_key)
            .send()
            .await
            .map_err(|e| io_err(&format!("delete_object {object_key}"), e))?;

        Ok(())
    }

    async fn list_keys(&self, project_id: &str) -> Result<Vec<String>, KvError> {
        let mut keys: Vec<String> = self
            .list_entries(project_id)
            .await?
            .into_iter()
            .map(|e| e.relative_key)
            .collect();
        keys.sort();
        Ok(keys)
    }

    async fn list_keys_with_sizes(&self, project_id: &str) -> Result<Vec<(String, u64)>, KvError> {
        let mut entries: Vec<(String, u64)> = self
            .list_entries(project_id)
            .await?
            .into_iter()
            .map(|e| (e.relative_key, e.size))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(entries)
    }

    async fn delete_all(&self, project_id: &str) -> Result<(), KvError> {
        let prefix = self.project_prefix(project_id);
        delete_all_under(&self.client, &self.bucket, &prefix)
            .await
            .map_err(|e| io_err(&format!("delete_all {prefix}"), e))?;
        Ok(())
    }

    async fn total_size(&self, project_id: &str) -> Result<u64, KvError> {
        Ok(self
            .list_entries(project_id)
            .await?
            .iter()
            .map(|e| e.size)
            .sum())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(prefix: &str) -> S3Settings {
        S3Settings {
            endpoint: "https://s3.example.invalid".to_owned(),
            region: "gra".to_owned(),
            bucket: "lwid".to_owned(),
            prefix: prefix.to_owned(),
            access_key_id: "ak".to_owned(),
            secret_access_key: "sk".to_owned(),
            force_path_style: true,
        }
    }

    #[test]
    fn prefix_normalisation() {
        assert_eq!(settings("").normalized_prefix(), "");
        assert_eq!(settings("/").normalized_prefix(), "");
        assert_eq!(settings("lwid").normalized_prefix(), "lwid/");
        assert_eq!(settings("/lwid/").normalized_prefix(), "lwid/");
        assert_eq!(settings("a/b").normalized_prefix(), "a/b/");
    }

    #[test]
    fn keys_match_the_filesystem_layout() {
        let client = client(&settings("data"));
        let cid = Cid::from_bytes(b"hello");

        let blobs = S3BlobStore::new(client.clone(), "lwid".to_owned(), "data/".to_owned());
        assert_eq!(blobs.key(&cid), format!("data/blobs/{}", cid.to_key()));

        let projects = S3ProjectStore::new(client.clone(), "lwid".to_owned(), "data/".to_owned());
        assert_eq!(projects.key("abc123"), "data/projects/abc123.json");
        assert_eq!(projects.list_prefix(), "data/projects/");

        let kv = S3KvStore::new(client, "lwid".to_owned(), "data/".to_owned());
        assert_eq!(kv.key("abc123", "a/b.txt"), "data/store/abc123/a/b.txt");
        assert_eq!(kv.project_prefix("abc123"), "data/store/abc123/");
    }

    #[test]
    fn empty_prefix_yields_bare_keys() {
        let client = client(&settings(""));
        let blobs = S3BlobStore::new(client, "lwid".to_owned(), String::new());
        let cid = Cid::from_bytes(b"hello");
        assert_eq!(blobs.key(&cid), format!("blobs/{}", cid.to_key()));
    }
}
