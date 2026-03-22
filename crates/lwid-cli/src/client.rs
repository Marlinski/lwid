//! HTTP client for the lookwhatidid API.

use lwid_common::wire::*;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("{}", extract_server_message(*.status, body))]
    Server { status: u16, body: String },

    #[allow(dead_code)]
    #[error("unexpected response: {0}")]
    Unexpected(String),
}

/// Extract a clean error message from server JSON responses.
/// Tries to parse `{"error": "..."}` and return just the message,
/// falling back to the raw body if parsing fails.
fn extract_server_message(status: u16, body: &str) -> String {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(msg) = json.get("error").and_then(|v| v.as_str()) {
            return format!("Error: {msg}");
        }
    }
    if body.is_empty() {
        format!("Server returned {status}")
    } else {
        format!("Error ({status}): {body}")
    }
}

pub struct Client {
    http: reqwest::Client,
    base_url: String,
    /// Bearer token for authenticated requests, if the user is logged in.
    token: Option<String>,
}

impl Client {
    pub fn new(base_url: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            token: crate::login::load_token(),
        }
    }

    /// Attach the `Authorization: Bearer <token>` header if a token is saved.
    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(ref tok) = self.token {
            req.header("Authorization", format!("Bearer {tok}"))
        } else {
            req
        }
    }

    /// POST /api/projects — create a new project.
    pub async fn create_project(
        &self,
        write_pubkey_b64: &str,
        ttl: Option<&str>,
        store_token: Option<&str>,
    ) -> Result<CreateProjectResponse, ApiError> {
        let resp = self
            .auth(self.http.post(format!("{}/api/projects", self.base_url)))
            .json(&CreateProjectRequest {
                write_pubkey: write_pubkey_b64.to_string(),
                ttl: ttl.map(|s| s.to_string()),
                store_token: store_token.map(|s| s.to_string()),
                client_version: Some(env!("LWID_VERSION").to_string()),
            })
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Server { status, body });
        }

        Ok(resp.json().await?)
    }

    /// GET /api/projects/{id} — get project metadata.
    pub async fn get_project(&self, id: &str) -> Result<ProjectResponse, ApiError> {
        let resp = self
            .auth(
                self.http
                    .get(format!("{}/api/projects/{}", self.base_url, id)),
            )
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Server { status, body });
        }

        Ok(resp.json().await?)
    }

    /// POST /api/blobs — upload a blob, returns the CID.
    pub async fn upload_blob(&self, data: Vec<u8>) -> Result<String, ApiError> {
        let resp = self
            .auth(self.http.post(format!("{}/api/blobs", self.base_url)))
            .body(data)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Server { status, body });
        }

        let upload: UploadBlobResponse = resp.json().await?;
        Ok(upload.cid)
    }

    /// HEAD /api/blobs/{cid} — check if blob exists.
    pub async fn blob_exists(&self, cid: &str) -> Result<bool, ApiError> {
        let resp = self
            .auth(
                self.http
                    .head(format!("{}/api/blobs/{}", self.base_url, cid)),
            )
            .send()
            .await?;

        Ok(resp.status().is_success())
    }

    /// GET /api/blobs/{cid} — download a blob.
    pub async fn get_blob(&self, cid: &str) -> Result<Vec<u8>, ApiError> {
        let resp = self
            .auth(
                self.http
                    .get(format!("{}/api/blobs/{}", self.base_url, cid)),
            )
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Server { status, body });
        }

        Ok(resp.bytes().await?.to_vec())
    }

    /// PUT /api/projects/{id}/root — update root CID with signature.
    pub async fn update_root(
        &self,
        id: &str,
        root_cid: &str,
        signature_b64: &str,
    ) -> Result<ProjectResponse, ApiError> {
        let resp = self
            .auth(
                self.http
                    .put(format!("{}/api/projects/{}/root", self.base_url, id)),
            )
            .json(&UpdateRootRequest {
                root_cid: root_cid.to_string(),
                signature: signature_b64.to_string(),
            })
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Server { status, body });
        }

        Ok(resp.json().await?)
    }

    /// PUT a value into the project store.
    pub async fn put_store_value(
        &self,
        project_id: &str,
        key: &str,
        value: &[u8],
        store_token: &str,
    ) -> Result<(), ApiError> {
        let url = format!(
            "{}/api/projects/{}/store/{}",
            self.base_url, project_id, key
        );
        let resp = self
            .auth(self.http.put(&url))
            .header("X-Store-Token", store_token)
            .header("Content-Type", "application/octet-stream")
            .body(value.to_vec())
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Server { status, body });
        }
        Ok(())
    }

    /// GET a value from the project store. Returns None if not found (404).
    pub async fn get_store_value(
        &self,
        project_id: &str,
        key: &str,
        store_token: &str,
    ) -> Result<Option<Vec<u8>>, ApiError> {
        let url = format!(
            "{}/api/projects/{}/store/{}",
            self.base_url, project_id, key
        );
        let resp = self
            .auth(self.http.get(&url))
            .header("X-Store-Token", store_token)
            .send()
            .await?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Server { status, body });
        }
        let bytes = resp.bytes().await?;
        Ok(Some(bytes.to_vec()))
    }

    /// DELETE a value from the project store.
    #[allow(dead_code)]
    pub async fn delete_store_value(
        &self,
        project_id: &str,
        key: &str,
        store_token: &str,
    ) -> Result<(), ApiError> {
        let url = format!(
            "{}/api/projects/{}/store/{}",
            self.base_url, project_id, key
        );
        let resp = self
            .auth(self.http.delete(&url))
            .header("X-Store-Token", store_token)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Server { status, body });
        }
        Ok(())
    }

    /// List all keys in the project store.
    pub async fn list_store_keys(
        &self,
        project_id: &str,
        store_token: &str,
    ) -> Result<StoreListResponse, ApiError> {
        let url = format!("{}/api/projects/{}/store", self.base_url, project_id);
        let resp = self
            .auth(self.http.get(&url))
            .header("X-Store-Token", store_token)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Server { status, body });
        }
        let result = resp.json().await?;
        Ok(result)
    }
}
