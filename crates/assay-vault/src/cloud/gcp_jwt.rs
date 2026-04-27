//! Google Cloud JWT signing for service-account auth.
//!
//! Used by [`crate::sealing::kms`] (GCP KMS auto-unseal) and the GCP
//! dynamic-creds provider (service-account impersonation via
//! `iamcredentials.googleapis.com`). Reuses `jsonwebtoken` already
//! in the workspace; no new crypto primitives.
//!
//! Flow:
//! 1. Build a JWT signed with the service-account private key
//!    (PKCS#8 PEM), `iss` = SA email, `aud` = OAuth2 token endpoint,
//!    `scope` = required cloud-API scope.
//! 2. POST the JWT as `assertion` to
//!    `https://oauth2.googleapis.com/token` with grant_type =
//!    `urn:ietf:params:oauth:grant-type:jwt-bearer`.
//! 3. Receive an OAuth2 access_token + expires_in.

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};

use crate::error::{Result, VaultError};

/// A Google Cloud service account loaded from the JSON key file.
#[derive(Clone, Debug, Deserialize)]
pub struct ServiceAccount {
    pub client_email: String,
    pub private_key: String,
    /// OAuth2 token URI; defaults to `https://oauth2.googleapis.com/token`.
    #[serde(default = "default_token_uri")]
    pub token_uri: String,
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".to_string()
}

#[derive(Serialize)]
struct Claims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    exp: u64,
    iat: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

/// One acquired access token with the absolute expiry epoch.
#[derive(Clone, Debug)]
pub struct AccessToken {
    pub access_token: String,
    pub expires_at: u64,
}

/// Acquire an OAuth2 access token for the given scope. The returned
/// token is valid for `expires_at` seconds (typically 3600).
#[cfg(feature = "vault-sealing-kms")]
pub async fn fetch_access_token(
    sa: &ServiceAccount,
    scope: &str,
) -> Result<AccessToken> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let claims = Claims {
        iss: &sa.client_email,
        scope,
        aud: &sa.token_uri,
        exp: now + 3600,
        iat: now,
    };
    let header = Header::new(Algorithm::RS256);
    let key = EncodingKey::from_rsa_pem(sa.private_key.as_bytes())
        .map_err(|e| VaultError::Crypto(format!("gcp jwt rsa pem: {e}")))?;
    let assertion = jsonwebtoken::encode(&header, &claims, &key)
        .map_err(|e| VaultError::Crypto(format!("gcp jwt encode: {e}")))?;

    let client = reqwest::Client::new();
    let resp = client
        .post(&sa.token_uri)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", &assertion),
        ])
        .send()
        .await
        .map_err(|e| VaultError::Backend(anyhow::anyhow!("gcp token POST: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(VaultError::Backend(anyhow::anyhow!(
            "gcp token endpoint returned {status}: {body}"
        )));
    }
    let token: TokenResponse = resp
        .json()
        .await
        .map_err(|e| VaultError::Backend(anyhow::anyhow!("gcp token decode: {e}")))?;
    Ok(AccessToken {
        access_token: token.access_token,
        expires_at: now + token.expires_in,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_account_default_token_uri() {
        let sa: ServiceAccount = serde_json::from_str(
            r#"{
                "client_email": "x@y.iam.gserviceaccount.com",
                "private_key": "..."
            }"#,
        )
        .unwrap();
        assert_eq!(sa.token_uri, "https://oauth2.googleapis.com/token");
    }
}
