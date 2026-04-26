//! AWS KMS auto-unseal — minimal sigv4 calls to the KMS Encrypt /
//! Decrypt API.
//!
//! Wire shape (TrentService.{Encrypt,Decrypt}):
//!
//! ```text
//! POST https://kms.{region}.amazonaws.com/
//! Content-Type: application/x-amz-json-1.1
//! X-Amz-Target: TrentService.Encrypt
//! Authorization: <sigv4>
//! {"KeyId": "alias/my-vault-key", "Plaintext": "<base64>"}
//! ```
//!
//! Returns `{"CiphertextBlob": "<base64>"}`. Decrypt is symmetric —
//! body `{"CiphertextBlob": "<base64>"}` returns `{"Plaintext": "..."}`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::cloud::sigv4::{now_amz_date, sign, SigV4Input};
use crate::crypto::sealing::KmsSeal;
use crate::error::{Result, VaultError};

/// Static IAM credentials for the KMS auto-unseal call. Operators
/// typically run the engine on EC2/Lambda with an IAM role; full IMDS
/// fetching is reserved for a follow-up — the canonical Phase-2
/// surface accepts explicit credentials so a localstack test path
/// works out of the box.
#[derive(Clone, Debug)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AwsKmsSeal {
    pub region: String,
    pub key_id: String,
    pub creds: AwsCredentials,
    /// Override the endpoint for tests / FIPS / VPC endpoints.
    pub endpoint_override: Option<String>,
    client: reqwest::Client,
}

#[derive(Serialize)]
struct EncryptBody<'a> {
    #[serde(rename = "KeyId")]
    key_id: &'a str,
    #[serde(rename = "Plaintext")]
    plaintext: String,
}

#[derive(Deserialize)]
struct EncryptResp {
    #[serde(rename = "CiphertextBlob")]
    ciphertext_blob: String,
}

#[derive(Serialize)]
struct DecryptBody<'a> {
    #[serde(rename = "CiphertextBlob")]
    ciphertext_blob: &'a str,
}

#[derive(Deserialize)]
struct DecryptResp {
    #[serde(rename = "Plaintext")]
    plaintext: String,
}

impl AwsKmsSeal {
    pub fn new(region: impl Into<String>, key_id: impl Into<String>, creds: AwsCredentials) -> Self {
        Self {
            region: region.into(),
            key_id: key_id.into(),
            creds,
            endpoint_override: None,
            client: reqwest::Client::new(),
        }
    }

    pub fn with_endpoint(mut self, ep: impl Into<String>) -> Self {
        self.endpoint_override = Some(ep.into());
        self
    }

    fn endpoint(&self) -> String {
        self.endpoint_override
            .clone()
            .unwrap_or_else(|| format!("https://kms.{}.amazonaws.com/", self.region))
    }

    async fn call(&self, target: &str, body: &[u8]) -> Result<Vec<u8>> {
        let amz_date = now_amz_date();
        let url = self.endpoint();
        let signed = sign(SigV4Input {
            access_key_id: &self.creds.access_key_id,
            secret_access_key: &self.creds.secret_access_key,
            session_token: self.creds.session_token.as_deref(),
            region: &self.region,
            service: "kms",
            method: "POST",
            url: &url,
            headers: &[
                ("content-type", "application/x-amz-json-1.1"),
                ("x-amz-target", target),
            ],
            body,
            amz_date: &amz_date,
        });

        let mut req = self.client.post(&signed.url).body(signed.body.clone());
        for (k, v) in &signed.headers {
            req = req.header(k, v);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("aws kms POST: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(VaultError::Backend(anyhow::anyhow!(
                "aws kms returned {status}: {body}"
            )));
        }
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("aws kms read body: {e}")))
    }
}

#[async_trait]
impl KmsSeal for AwsKmsSeal {
    async fn wrap_kek(&self, raw: &[u8]) -> Result<Vec<u8>> {
        let body = serde_json::to_vec(&EncryptBody {
            key_id: &self.key_id,
            plaintext: data_encoding::BASE64.encode(raw),
        })
        .map_err(|e| VaultError::Backend(anyhow::anyhow!("encode encrypt body: {e}")))?;
        let raw = self.call("TrentService.Encrypt", &body).await?;
        let parsed: EncryptResp = serde_json::from_slice(&raw)
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("decode encrypt resp: {e}")))?;
        data_encoding::BASE64
            .decode(parsed.ciphertext_blob.as_bytes())
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("decode ciphertext_blob b64: {e}")))
    }

    async fn unwrap_kek(&self, wrapped: &[u8]) -> Result<Vec<u8>> {
        let body = serde_json::to_vec(&DecryptBody {
            ciphertext_blob: &data_encoding::BASE64.encode(wrapped),
        })
        .map_err(|e| VaultError::Backend(anyhow::anyhow!("encode decrypt body: {e}")))?;
        let raw = self.call("TrentService.Decrypt", &body).await?;
        let parsed: DecryptResp = serde_json::from_slice(&raw)
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("decode decrypt resp: {e}")))?;
        data_encoding::BASE64
            .decode(parsed.plaintext.as_bytes())
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("decode plaintext b64: {e}")))
    }

    fn identifier(&self) -> String {
        format!("aws-kms:{}:{}", self.region, self.key_id)
    }
}
