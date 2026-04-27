//! Google Cloud KMS auto-unseal — Cloud KMS REST API.
//!
//! Wire shape:
//!
//! ```text
//! POST https://cloudkms.googleapis.com/v1/projects/{p}/locations/{l}/keyRings/{r}/cryptoKeys/{k}:encrypt
//! Authorization: Bearer <oauth2_token>
//! Content-Type: application/json
//! {"plaintext": "<base64>"}
//! ```
//!
//! Returns `{"ciphertext": "<base64>"}`. Decrypt mirrors with `:decrypt`
//! and `{"ciphertext": "..."}` in/out.

use async_trait::async_trait;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::cloud::gcp_jwt::{fetch_access_token, AccessToken, ServiceAccount};
use crate::crypto::sealing::KmsSeal;
use crate::error::{Result, VaultError};

const KMS_SCOPE: &str = "https://www.googleapis.com/auth/cloudkms";

#[derive(Clone, Debug)]
pub struct GcpKmsSeal {
    pub project: String,
    pub location: String,
    pub key_ring: String,
    pub key: String,
    sa: Arc<ServiceAccount>,
    cached_token: Arc<RwLock<Option<AccessToken>>>,
    client: reqwest::Client,
}

#[derive(Serialize)]
struct EncryptBody<'a> {
    plaintext: &'a str,
}

#[derive(Deserialize)]
struct EncryptResp {
    ciphertext: String,
}

#[derive(Serialize)]
struct DecryptBody<'a> {
    ciphertext: &'a str,
}

#[derive(Deserialize)]
struct DecryptResp {
    plaintext: String,
}

impl GcpKmsSeal {
    pub fn new(
        project: impl Into<String>,
        location: impl Into<String>,
        key_ring: impl Into<String>,
        key: impl Into<String>,
        sa: ServiceAccount,
    ) -> Self {
        Self {
            project: project.into(),
            location: location.into(),
            key_ring: key_ring.into(),
            key: key.into(),
            sa: Arc::new(sa),
            cached_token: Arc::new(RwLock::new(None)),
            client: reqwest::Client::new(),
        }
    }

    fn key_path(&self) -> String {
        format!(
            "projects/{}/locations/{}/keyRings/{}/cryptoKeys/{}",
            self.project, self.location, self.key_ring, self.key
        )
    }

    fn endpoint_for(&self, op: &str) -> String {
        format!(
            "https://cloudkms.googleapis.com/v1/{}:{}",
            self.key_path(),
            op
        )
    }

    async fn token(&self) -> Result<String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if let Some(t) = self.cached_token.read().clone() {
            // Refresh ~60s before expiry so a request near the boundary
            // doesn't fail mid-flight.
            if t.expires_at > now + 60 {
                return Ok(t.access_token);
            }
        }
        let fresh = fetch_access_token(&self.sa, KMS_SCOPE).await?;
        let token_str = fresh.access_token.clone();
        *self.cached_token.write() = Some(fresh);
        Ok(token_str)
    }

    async fn call_kms(&self, op: &str, body: &[u8]) -> Result<Vec<u8>> {
        let token = self.token().await?;
        let url = self.endpoint_for(op);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .header("content-type", "application/json")
            .body(body.to_vec())
            .send()
            .await
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("gcp kms POST: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(VaultError::Backend(anyhow::anyhow!(
                "gcp kms {op} returned {status}: {txt}"
            )));
        }
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("gcp kms read body: {e}")))
    }
}

#[async_trait]
impl KmsSeal for GcpKmsSeal {
    async fn wrap_kek(&self, raw: &[u8]) -> Result<Vec<u8>> {
        let body = serde_json::to_vec(&EncryptBody {
            plaintext: &data_encoding::BASE64.encode(raw),
        })
        .map_err(|e| VaultError::Backend(anyhow::anyhow!("encode encrypt body: {e}")))?;
        let raw = self.call_kms("encrypt", &body).await?;
        let parsed: EncryptResp = serde_json::from_slice(&raw)
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("decode encrypt resp: {e}")))?;
        data_encoding::BASE64
            .decode(parsed.ciphertext.as_bytes())
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("decode ciphertext b64: {e}")))
    }

    async fn unwrap_kek(&self, wrapped: &[u8]) -> Result<Vec<u8>> {
        let body = serde_json::to_vec(&DecryptBody {
            ciphertext: &data_encoding::BASE64.encode(wrapped),
        })
        .map_err(|e| VaultError::Backend(anyhow::anyhow!("encode decrypt body: {e}")))?;
        let raw = self.call_kms("decrypt", &body).await?;
        let parsed: DecryptResp = serde_json::from_slice(&raw)
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("decode decrypt resp: {e}")))?;
        data_encoding::BASE64
            .decode(parsed.plaintext.as_bytes())
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("decode plaintext b64: {e}")))
    }

    fn identifier(&self) -> String {
        format!(
            "gcp-kms:{}/{}/{}/{}",
            self.project, self.location, self.key_ring, self.key
        )
    }
}
