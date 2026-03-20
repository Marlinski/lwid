//! End-to-end integration tests for the LookWhatIDid API.
//!
//! These tests exercise the full create → upload → update-root → fetch flow
//! using an in-process axum test server backed by temporary directories.

use std::sync::Arc;

use axum::http::StatusCode;
use axum_test::TestServer;
use base64::prelude::*;
use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use serde_json::{json, Value};
use tempfile::TempDir;

use lwid_server::api::{self, AppState};
use lwid_server::config::Config;
use lwid_common::kv::FsKvStore;
use lwid_common::project::FsProjectStore;
use lwid_common::store::FsBlobStore;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Spin up a test server backed by temp directories.
fn test_server() -> (TestServer, TempDir) {
    let tmp = TempDir::new().expect("create temp dir");

    let blob_dir = tmp.path().join("blobs");
    let project_dir = tmp.path().join("projects");
    let kv_dir = tmp.path().join("store");
    let shell_dir = tmp.path().join("shell");

    // Create a minimal shell dir with an index.html for static serving tests
    std::fs::create_dir_all(&shell_dir).unwrap();
    std::fs::write(shell_dir.join("index.html"), "<html>test</html>").unwrap();

    let blob_store = FsBlobStore::new(blob_dir).expect("create blob store");
    let project_store = FsProjectStore::new(project_dir).expect("create project store");
    let kv_store = FsKvStore::new(kv_dir).expect("create kv store");

    let mut config = Config::default();
    config.server.shell_dir = shell_dir;

    let state = AppState {
        blobs: Arc::new(blob_store),
        projects: Arc::new(project_store),
        kv: Arc::new(kv_store),
        config,
    };

    let app = api::router(state);
    let server = TestServer::new(app);
    (server, tmp)
}

/// Generate an Ed25519 signing key and return (signing_key, base64_pubkey).
fn generate_keypair() -> (SigningKey, String) {
    let signing_key = SigningKey::generate(&mut OsRng);
    let pubkey_bytes = signing_key.verifying_key().to_bytes();
    let pubkey_b64 = BASE64_STANDARD.encode(pubkey_bytes);
    (signing_key, pubkey_b64)
}

/// Build a minimal valid manifest JSON.
fn build_manifest(version: u64, parent_cid: Option<&str>, files: &[(&str, &str, usize)]) -> Value {
    let files: Vec<Value> = files
        .iter()
        .map(|(path, cid, size)| {
            json!({ "path": path, "cid": cid, "size": size })
        })
        .collect();

    json!({
        "version": version,
        "parent_cid": parent_cid,
        "timestamp": "2026-03-20T12:00:00Z",
        "files": files,
    })
}

/// Upload a manifest blob and return its CID string.
async fn upload_manifest(server: &TestServer, manifest: &Value) -> String {
    let bytes = serde_json::to_vec(manifest).unwrap();
    let resp = server.post("/api/blobs").bytes(bytes.into()).await;
    resp.assert_status_ok();
    resp.json::<Value>()["cid"].as_str().unwrap().to_string()
}

/// Sign a CID string and update the project root, asserting success.
async fn sign_and_update_root(
    server: &TestServer,
    project_id: &str,
    root_cid: &str,
    signing_key: &SigningKey,
) -> Value {
    let signature = signing_key.sign(root_cid.as_bytes());
    let sig_b64 = BASE64_STANDARD.encode(signature.to_bytes());

    let resp = server
        .put(&format!("/api/projects/{project_id}/root"))
        .json(&json!({
            "root_cid": root_cid,
            "signature": sig_b64,
        }))
        .await;
    resp.assert_status_ok();
    resp.json()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn full_lifecycle_create_push_fetch() {
    let (server, _tmp) = test_server();
    let (signing_key, pubkey_b64) = generate_keypair();

    // 1. Create a project
    let create_resp = server
        .post("/api/projects")
        .json(&json!({ "write_pubkey": pubkey_b64 }))
        .await;

    create_resp.assert_status(StatusCode::CREATED);
    let body: Value = create_resp.json();
    let project_id = body["project_id"].as_str().unwrap().to_string();
    assert!(!project_id.is_empty());

    // 2. Upload blobs (simulating encrypted files)
    let blob1_data = b"<html><body>Hello encrypted world</body></html>";
    let blob2_data = b"body { background: #000; color: #fff; }";

    let resp1 = server
        .post("/api/blobs")
        .bytes(blob1_data.to_vec().into())
        .await;
    resp1.assert_status_ok();
    let cid1_str = resp1.json::<Value>()["cid"].as_str().unwrap().to_string();

    let resp2 = server
        .post("/api/blobs")
        .bytes(blob2_data.to_vec().into())
        .await;
    resp2.assert_status_ok();
    let cid2_str = resp2.json::<Value>()["cid"].as_str().unwrap().to_string();

    // 3. Upload a manifest blob (plaintext JSON referencing the file blobs)
    let manifest = build_manifest(
        1,
        None,
        &[
            ("index.html", &cid1_str, blob1_data.len()),
            ("style.css", &cid2_str, blob2_data.len()),
        ],
    );
    let root_cid = upload_manifest(&server, &manifest).await;

    // 4. Update the project root with a signed request
    let updated = sign_and_update_root(&server, &project_id, &root_cid, &signing_key).await;
    assert_eq!(updated["root_cid"].as_str().unwrap(), root_cid);

    // 5. Fetch the project metadata and verify the root CID is set
    let get_resp = server
        .get(&format!("/api/projects/{project_id}"))
        .await;
    get_resp.assert_status_ok();
    let project: Value = get_resp.json();
    assert_eq!(project["id"].as_str().unwrap(), project_id);
    assert_eq!(project["root_cid"].as_str().unwrap(), root_cid);

    // 6. Fetch the manifest blob back
    let manifest_get = server.get(&format!("/api/blobs/{root_cid}")).await;
    manifest_get.assert_status_ok();
    let fetched_manifest: Value = serde_json::from_slice(manifest_get.as_bytes()).unwrap();
    assert_eq!(fetched_manifest["version"], 1);
    assert_eq!(fetched_manifest["files"].as_array().unwrap().len(), 2);

    // 7. Fetch each file blob and verify content
    let blob1_get = server.get(&format!("/api/blobs/{cid1_str}")).await;
    blob1_get.assert_status_ok();
    assert_eq!(blob1_get.as_bytes().as_ref(), blob1_data);

    let blob2_get = server.get(&format!("/api/blobs/{cid2_str}")).await;
    blob2_get.assert_status_ok();
    assert_eq!(blob2_get.as_bytes().as_ref(), blob2_data);
}

#[tokio::test]
async fn create_project_with_ttl() {
    let (server, _tmp) = test_server();
    let (_, pubkey_b64) = generate_keypair();

    // Default TTL (7d)
    let resp = server
        .post("/api/projects")
        .json(&json!({ "write_pubkey": pubkey_b64 }))
        .await;
    resp.assert_status(StatusCode::CREATED);
    let body: Value = resp.json();
    let project_id = body["project_id"].as_str().unwrap();

    let get_resp = server
        .get(&format!("/api/projects/{project_id}"))
        .await;
    let project: Value = get_resp.json();
    assert!(
        project["expires_at"].as_str().is_some(),
        "default TTL should set expires_at",
    );

    // Explicit TTL = "never"
    let (_, pubkey_b64_2) = generate_keypair();
    let resp2 = server
        .post("/api/projects")
        .json(&json!({ "write_pubkey": pubkey_b64_2, "ttl": "never" }))
        .await;
    resp2.assert_status(StatusCode::CREATED);
    let body2: Value = resp2.json();
    let project_id2 = body2["project_id"].as_str().unwrap();

    let get_resp2 = server
        .get(&format!("/api/projects/{project_id2}"))
        .await;
    let project2: Value = get_resp2.json();
    assert!(
        project2["expires_at"].is_null(),
        "TTL 'never' should not set expires_at",
    );

    // Invalid TTL
    let (_, pubkey_b64_3) = generate_keypair();
    let resp3 = server
        .post("/api/projects")
        .json(&json!({ "write_pubkey": pubkey_b64_3, "ttl": "99y" }))
        .await;
    resp3.assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn update_root_wrong_key_is_rejected() {
    let (server, _tmp) = test_server();
    let (_, pubkey_b64) = generate_keypair();
    let (wrong_key, _) = generate_keypair();

    // Create a project
    let create_resp = server
        .post("/api/projects")
        .json(&json!({ "write_pubkey": pubkey_b64 }))
        .await;
    create_resp.assert_status(StatusCode::CREATED);
    let body: Value = create_resp.json();
    let project_id = body["project_id"].as_str().unwrap();

    // Upload a valid manifest blob
    let manifest = build_manifest(1, None, &[]);
    let root_cid = upload_manifest(&server, &manifest).await;

    // Sign with the wrong key
    let bad_sig = wrong_key.sign(root_cid.as_bytes());
    let sig_b64 = BASE64_STANDARD.encode(bad_sig.to_bytes());

    let update_resp = server
        .put(&format!("/api/projects/{project_id}/root"))
        .json(&json!({
            "root_cid": root_cid,
            "signature": sig_b64,
        }))
        .await;

    update_resp.assert_status_unauthorized();
}

#[tokio::test]
async fn update_root_rejects_invalid_manifest() {
    let (server, _tmp) = test_server();
    let (signing_key, pubkey_b64) = generate_keypair();

    // Create a project
    let create_resp = server
        .post("/api/projects")
        .json(&json!({ "write_pubkey": pubkey_b64 }))
        .await;
    let body: Value = create_resp.json();
    let project_id = body["project_id"].as_str().unwrap();

    // Upload a non-JSON blob
    let resp = server
        .post("/api/blobs")
        .bytes(b"this is not json".to_vec().into())
        .await;
    let root_cid = resp.json::<Value>()["cid"].as_str().unwrap().to_string();

    // Sign and try to update root — should fail because the blob is not valid
    // manifest JSON
    let signature = signing_key.sign(root_cid.as_bytes());
    let sig_b64 = BASE64_STANDARD.encode(signature.to_bytes());

    let update_resp = server
        .put(&format!("/api/projects/{project_id}/root"))
        .json(&json!({
            "root_cid": root_cid,
            "signature": sig_b64,
        }))
        .await;

    update_resp.assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn version_chain_two_pushes() {
    let (server, _tmp) = test_server();
    let (signing_key, pubkey_b64) = generate_keypair();

    // Create project
    let create_resp = server
        .post("/api/projects")
        .json(&json!({ "write_pubkey": pubkey_b64 }))
        .await;
    let body: Value = create_resp.json();
    let project_id = body["project_id"].as_str().unwrap().to_string();

    // Push version 1
    let v1_manifest = build_manifest(1, None, &[]);
    let v1_cid = upload_manifest(&server, &v1_manifest).await;
    sign_and_update_root(&server, &project_id, &v1_cid, &signing_key).await;

    // Push version 2 (with parent pointing to v1)
    let v2_manifest = build_manifest(2, Some(&v1_cid), &[]);
    let v2_cid = upload_manifest(&server, &v2_manifest).await;
    sign_and_update_root(&server, &project_id, &v2_cid, &signing_key).await;

    // Verify root points to v2
    let project: Value = server
        .get(&format!("/api/projects/{project_id}"))
        .await
        .json();
    assert_eq!(project["root_cid"].as_str().unwrap(), v2_cid);

    // Fetch v2 manifest and verify parent chain
    let v2_blob = server.get(&format!("/api/blobs/{v2_cid}")).await;
    let v2_data: Value = serde_json::from_slice(v2_blob.as_bytes()).unwrap();
    assert_eq!(v2_data["version"], 2);
    assert_eq!(v2_data["parent_cid"].as_str().unwrap(), v1_cid);

    // Fetch v1 manifest via parent link
    let v1_blob = server.get(&format!("/api/blobs/{v1_cid}")).await;
    let v1_data: Value = serde_json::from_slice(v1_blob.as_bytes()).unwrap();
    assert_eq!(v1_data["version"], 1);
    assert!(v1_data["parent_cid"].is_null());
}

#[tokio::test]
async fn static_serving_works() {
    let (server, _tmp) = test_server();

    // GET / should serve index.html
    let resp = server.get("/").await;
    resp.assert_status_ok();
    let body = resp.text();
    assert!(body.contains("<html>test</html>"));

    // GET /p/some-project should also serve index.html (SPA fallback)
    let resp = server.get("/p/some-project-id").await;
    resp.assert_status_ok();
    let body = resp.text();
    assert!(body.contains("<html>test</html>"));
}

#[tokio::test]
async fn blob_too_large_is_rejected() {
    let (server, _tmp) = test_server();

    // Default max is 10MB, upload something just over
    let big_data = vec![0u8; 10 * 1024 * 1024 + 1];
    let resp = server
        .post("/api/blobs")
        .bytes(big_data.into())
        .await;

    // Should be 413 Payload Too Large
    resp.assert_status(StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn idempotent_blob_upload() {
    let (server, _tmp) = test_server();

    let data = b"same content twice";

    let resp1 = server
        .post("/api/blobs")
        .bytes(data.to_vec().into())
        .await;
    let cid1 = resp1.json::<Value>()["cid"]
        .as_str()
        .unwrap()
        .to_string();

    let resp2 = server
        .post("/api/blobs")
        .bytes(data.to_vec().into())
        .await;
    let cid2 = resp2.json::<Value>()["cid"]
        .as_str()
        .unwrap()
        .to_string();

    // Same content = same CID (content-addressed, dedup)
    assert_eq!(cid1, cid2);
}
