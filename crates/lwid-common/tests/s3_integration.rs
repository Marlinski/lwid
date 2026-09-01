//! Integration tests for the S3 backends against a live S3-compatible server.
//!
//! These are skipped unless `LWID_TEST_S3_ENDPOINT` is set, so an ordinary
//! `cargo test` stays hermetic. To run them, start any S3 implementation and
//! point the suite at it:
//!
//! ```sh
//! docker run -d --rm -p 9000:9000 \
//!   -e MINIO_ROOT_USER=lwidtest -e MINIO_ROOT_PASSWORD=lwidtest123 \
//!   minio/minio server /data
//! mc alias set t http://127.0.0.1:9000 lwidtest lwidtest123 && mc mb t/lwid
//!
//! LWID_TEST_S3_ENDPOINT=http://127.0.0.1:9000 \
//! LWID_TEST_S3_BUCKET=lwid \
//! LWID_TEST_S3_ACCESS_KEY=lwidtest \
//! LWID_TEST_S3_SECRET_KEY=lwidtest123 \
//!   cargo test -p lwid-common --features s3 --test s3_integration
//! ```
#![cfg(feature = "s3")]

use std::collections::BTreeSet;

use futures::stream::StreamExt;

use lwid_common::cid::Cid;
use lwid_common::kv::{KvError, KvStore};
use lwid_common::project::{ProjectError, ProjectStore};
use lwid_common::s3::{client, S3BlobStore, S3KvStore, S3ProjectStore, S3Settings};
use lwid_common::store::{BlobStore, StoreError};

/// How many object-storage requests to keep in flight in the bulk tests.
///
/// Kept deliberately low: unbounded fan-out exhausts the connection pool
/// (every request fails to dispatch) and can overwhelm a small local server.
const CONCURRENCY: usize = 8;

/// Build settings from the environment, or `None` when the suite is disabled.
///
/// Each test gets its own key prefix so runs cannot collide.
fn settings(test_name: &str) -> Option<S3Settings> {
    let endpoint = std::env::var("LWID_TEST_S3_ENDPOINT").ok()?;
    Some(S3Settings {
        endpoint,
        region: std::env::var("LWID_TEST_S3_REGION").unwrap_or_else(|_| "us-east-1".to_owned()),
        bucket: std::env::var("LWID_TEST_S3_BUCKET").unwrap_or_else(|_| "lwid".to_owned()),
        prefix: format!("it/{test_name}"),
        access_key_id: std::env::var("LWID_TEST_S3_ACCESS_KEY")
            .unwrap_or_else(|_| "lwidtest".to_owned()),
        secret_access_key: std::env::var("LWID_TEST_S3_SECRET_KEY")
            .unwrap_or_else(|_| "lwidtest123".to_owned()),
        force_path_style: true,
    })
}

/// Expand to an early `return` when the suite is not configured.
macro_rules! stores {
    ($name:literal) => {
        match settings($name) {
            Some(s) => {
                let c = client(&s);
                let prefix = s.normalized_prefix();
                (
                    S3BlobStore::new(c.clone(), s.bucket.clone(), prefix.clone()),
                    S3ProjectStore::new(c.clone(), s.bucket.clone(), prefix.clone()),
                    S3KvStore::new(c, s.bucket.clone(), prefix),
                )
            }
            None => {
                eprintln!("skipping {}: LWID_TEST_S3_ENDPOINT not set", $name);
                return;
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Blob store
// ---------------------------------------------------------------------------

#[tokio::test]
async fn blob_roundtrip_and_not_found() {
    let (blobs, _, _) = stores!("blob_roundtrip");

    let data = b"content-addressed payload".to_vec();
    let cid = blobs.put(&data).await.expect("put");

    assert_eq!(blobs.get(&cid).await.expect("get"), data);
    assert!(blobs.exists(&cid).await.expect("exists"));

    // Idempotent: same bytes, same CID, no error.
    assert_eq!(blobs.put(&data).await.expect("put again"), cid);

    let absent = Cid::from_bytes(b"never uploaded");
    assert!(!blobs.exists(&absent).await.expect("exists on missing"));
    assert!(matches!(
        blobs.get(&absent).await,
        Err(StoreError::NotFound { .. })
    ));
    assert!(
        matches!(blobs.delete(&absent).await, Err(StoreError::NotFound { .. })),
        "deleting a missing blob must report NotFound, not silently succeed",
    );

    blobs.delete(&cid).await.expect("delete");
    assert!(!blobs.exists(&cid).await.expect("exists after delete"));
}

#[tokio::test]
async fn blob_handles_binary_and_empty_payloads() {
    let (blobs, _, _) = stores!("blob_payloads");

    let empty = blobs.put(b"").await.expect("put empty");
    assert_eq!(blobs.get(&empty).await.expect("get empty"), b"");

    let binary: Vec<u8> = (0..=255u8).cycle().take(64 * 1024).collect();
    let cid = blobs.put(&binary).await.expect("put binary");
    assert_eq!(blobs.get(&cid).await.expect("get binary"), binary);

    blobs.delete(&empty).await.ok();
    blobs.delete(&cid).await.ok();
}

// ---------------------------------------------------------------------------
// Project store
// ---------------------------------------------------------------------------

#[tokio::test]
async fn project_lifecycle() {
    let (_, projects, _) = stores!("project_lifecycle");

    let pubkey = [7u8; 32];
    let created = projects
        .create(&pubkey, None, Some("token".to_owned()), Some("test".to_owned()))
        .await
        .expect("create");

    let fetched = projects.get(&created.id).await.expect("get");
    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.write_pubkey, pubkey.to_vec());
    assert_eq!(fetched.store_token.as_deref(), Some("token"));
    assert!(fetched.root_cid.is_none());

    let cid = Cid::from_bytes(b"a manifest");
    let mut blob_cids = BTreeSet::new();
    blob_cids.insert(cid.to_string());
    let updated = projects
        .update_root(&created.id, cid.clone(), blob_cids.clone())
        .await
        .expect("update_root");
    assert_eq!(updated.root_cid.as_ref(), Some(&cid));
    assert_eq!(updated.blob_cids, blob_cids);

    // Persisted, not just returned.
    let refetched = projects.get(&created.id).await.expect("get after update");
    assert_eq!(refetched.root_cid.as_ref(), Some(&cid));

    let expiry = chrono::Utc::now() + chrono::TimeDelta::days(1);
    let extended = projects
        .update_expiry(&created.id, Some(expiry))
        .await
        .expect("update_expiry");
    assert!(extended.expires_at.is_some());

    assert!(
        projects.list().await.expect("list").contains(&created.id),
        "created project must appear in list()",
    );

    projects.delete(&created.id).await.expect("delete");
    assert!(matches!(
        projects.get(&created.id).await,
        Err(ProjectError::NotFound { .. })
    ));
    assert!(matches!(
        projects.get("nonexistent-id").await,
        Err(ProjectError::NotFound { .. })
    ));
}

// ---------------------------------------------------------------------------
// KV store
// ---------------------------------------------------------------------------

#[tokio::test]
async fn kv_lifecycle_and_nested_keys() {
    let (_, _, kv) = stores!("kv_lifecycle");
    let pid = "proj-kv";

    kv.delete_all(pid).await.expect("clean slate");

    kv.put(pid, "greeting", b"hello").await.expect("put");
    kv.put(pid, "a/b/c.txt", b"nested").await.expect("put nested");

    assert_eq!(kv.get(pid, "greeting").await.expect("get"), b"hello");
    assert_eq!(kv.list_keys(pid).await.expect("list"), vec!["a/b/c.txt", "greeting"]);
    assert_eq!(kv.total_size(pid).await.expect("total_size"), 11);

    let sizes = kv.list_keys_with_sizes(pid).await.expect("sizes");
    assert_eq!(sizes, vec![("a/b/c.txt".into(), 6), ("greeting".into(), 5)]);

    // Overwrite must replace, not accumulate.
    kv.put(pid, "greeting", b"hi").await.expect("overwrite");
    assert_eq!(kv.get(pid, "greeting").await.expect("get"), b"hi");
    assert_eq!(kv.total_size(pid).await.expect("total_size"), 8);

    assert!(matches!(
        kv.get(pid, "missing").await,
        Err(KvError::NotFound { .. })
    ));
    assert!(
        matches!(kv.delete(pid, "missing").await, Err(KvError::NotFound { .. })),
        "deleting a missing key must report NotFound, not silently succeed",
    );
    assert!(matches!(
        kv.put(pid, "../escape", b"x").await,
        Err(KvError::InvalidKey { .. })
    ));

    kv.delete(pid, "greeting").await.expect("delete");
    assert_eq!(kv.list_keys(pid).await.expect("list"), vec!["a/b/c.txt"]);

    kv.delete_all(pid).await.expect("delete_all");
    assert!(kv.list_keys(pid).await.expect("list").is_empty());
    assert_eq!(kv.total_size(pid).await.expect("total_size"), 0);
}

/// The one behaviour a small bucket never reaches: a listing that spans more
/// than one `ListObjectsV2` page. S3 caps a page at 1000 keys, so anything
/// that lists must follow continuation tokens or silently truncate.
#[tokio::test]
async fn kv_listing_spans_multiple_pages() {
    let (_, _, kv) = stores!("kv_pagination");
    let pid = "proj-pagination";
    const COUNT: usize = 1005;

    kv.delete_all(pid).await.expect("clean slate");

    // Write with bounded concurrency: 1005 sequential round trips is needlessly
    // slow, but firing all of them at once exhausts the connection pool.
    // The keys must outlive the futures that borrow them.
    let names: Vec<String> = (0..COUNT).map(|i| format!("key-{i:05}")).collect();
    let results: Vec<_> = futures::stream::iter(names.iter().map(|name| kv.put(pid, name, b"x")))
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;
    for (name, result) in names.iter().zip(results) {
        result.unwrap_or_else(|e| panic!("put {name} failed: {e}"));
    }

    let keys = kv.list_keys(pid).await.expect("list");
    assert_eq!(
        keys.len(),
        COUNT,
        "listing must follow continuation tokens past the 1000-key page limit",
    );
    assert_eq!(keys.first().map(String::as_str), Some("key-00000"));
    assert_eq!(keys.last().map(String::as_str), Some("key-01004"));
    assert_eq!(kv.total_size(pid).await.expect("total_size"), COUNT as u64);

    // Batch delete must also cover every page (S3 takes 1000 keys per call).
    kv.delete_all(pid).await.expect("delete_all");
    assert!(
        kv.list_keys(pid).await.expect("list after delete_all").is_empty(),
        "delete_all must remove every key, including those past the first page",
    );
}

/// Projects live under a shared prefix, so `list()` pages the same way.
#[tokio::test]
async fn project_list_spans_multiple_pages() {
    let (_, projects, _) = stores!("project_pagination");
    const COUNT: usize = 1002;

    let pubkey = [3u8; 32];
    let created: Vec<String> =
        futures::stream::iter((0..COUNT).map(|_| projects.create(&pubkey, None, None, None)))
            .buffer_unordered(CONCURRENCY)
            .map(|r| r.expect("create").id)
            .collect()
            .await;

    let listed = projects.list().await.expect("list");
    assert_eq!(
        listed.len(),
        COUNT,
        "project listing must follow continuation tokens",
    );

    let _: Vec<_> = futures::stream::iter(created.iter().map(|id| projects.delete(id)))
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;
}
