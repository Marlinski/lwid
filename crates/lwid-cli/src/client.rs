//! HTTP client for the lookwhatidid API.

use lwid_common::wire::*;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("server error ({status}): {body}")]
    Server { status: u16, body: String },

    #[error("unexpected response: {0}")]
    Unexpected(String),
}

pub struct Client {
    http: reqwest::Client,
    base_url: String,
}

impl Client {
    pub fn new(base_url: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// POST /api/projects — create a new project.
    pub async fn create_project(
        &self,
        write_pubkey_b64: &str,
    ) -> Result<CreateProjectResponse, ApiError> {
        let resp = self
            .http
            .post(format!("{}/api/projects", self.base_url))
            .json(&CreateProjectRequest {
                write_pubkey: write_pubkey_b64.to_string(),
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
            .http
            .get(format!("{}/api/projects/{}", self.base_url, id))
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
            .http
            .post(format!("{}/api/blobs", self.base_url))
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
            .http
            .head(format!("{}/api/blobs/{}", self.base_url, cid))
            .send()
            .await?;

        Ok(resp.status().is_success())
    }

    /// GET /api/blobs/{cid} — download a blob.
    pub async fn get_blob(&self, cid: &str) -> Result<Vec<u8>, ApiError> {
        let resp = self
            .http
            .get(format!("{}/api/blobs/{}", self.base_url, cid))
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
            .http
            .put(format!("{}/api/projects/{}/root", self.base_url, id))
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
}
