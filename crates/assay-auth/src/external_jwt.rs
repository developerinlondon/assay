//! External JWT issuer pass-through validation.
//!
//! When configured, the engine accepts `Authorization: Bearer <jwt>`
//! tokens minted by an upstream OIDC provider (Hydra, Keycloak,
//! Auth0, …) without managing its own users. Each issuer's JWKS is
//! discovered once via `<issuer>/.well-known/openid-configuration`,
//! cached in memory, and used to verify incoming tokens at request
//! time. Tokens carrying an `iss` outside the configured list are
//! rejected — they fall through to the existing internal-JWT path.
//!
//! This is the v0.3.2 restoration of v0.12.1's `--auth-issuer` /
//! `--auth-audience` behavior, configured via TOML instead of CLI
//! flags. See the `[[auth.external_issuers]]` block in `engine.toml`.
//!
//! ## What it is
//!
//! Pass-through validation: the upstream IdP is the source of truth
//! for users and sessions; the engine just verifies signatures and
//! claims and treats the request as authenticated. Zero schema
//! impact, no user-table writes, no engine-managed sessions.
//!
//! ## What it isn't
//!
//! This is **not** OIDC federation — there's no `/login/<slug>`
//! redirect, no PKCE, no callback. For that use case see
//! [`crate::oidc::OidcRegistry`] (operators wanting the engine to
//! own login flow). Pass-through is for deployments where the
//! engine sits behind another service that already terminates auth
//! and forwards the JWT.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use jsonwebtoken::dangerous::insecure_decode;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{DecodingKey, Validation, decode, decode_header};
use parking_lot::RwLock;
use serde::de::DeserializeOwned;

use crate::error::{Error, Result};

/// Verifier for one external OIDC issuer. Holds a cached JWKS plus the
/// claims policy (`iss`, `aud`) the operator configured. Construct via
/// [`ExternalJwtIssuer::discover`] at engine boot; clone freely (the
/// JWKS sits behind an `Arc<RwLock>`).
#[derive(Clone)]
pub struct ExternalJwtIssuer {
    issuer_url: String,
    audience: HashSet<String>,
    jwks_uri: String,
    jwks: Arc<RwLock<JwkSet>>,
    refresh_interval: Duration,
}

impl ExternalJwtIssuer {
    /// Discover the issuer's metadata (`<issuer_url>/.well-known/openid-configuration`),
    /// fetch the initial JWKS, and return a verifier ready for use.
    /// Spawns a background task that refreshes the JWKS every
    /// `refresh_secs` seconds — handles upstream key rotation without
    /// operator intervention.
    pub async fn discover(
        issuer_url: String,
        audience: Vec<String>,
        refresh_secs: u64,
    ) -> Result<Self> {
        let trimmed = issuer_url.trim_end_matches('/').to_string();
        let discovery_url = format!("{trimmed}/.well-known/openid-configuration");

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| Error::Oidc(format!("build http client: {e}")))?;

        let metadata: serde_json::Value = client
            .get(&discovery_url)
            .send()
            .await
            .map_err(|e| Error::Oidc(format!("discover {discovery_url}: {e}")))?
            .error_for_status()
            .map_err(|e| Error::Oidc(format!("discover {discovery_url}: {e}")))?
            .json()
            .await
            .map_err(|e| Error::Oidc(format!("parse {discovery_url}: {e}")))?;

        let jwks_uri = metadata
            .get("jwks_uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Oidc(format!("{discovery_url} missing `jwks_uri` field")))?
            .to_string();

        let jwks = fetch_jwks(&client, &jwks_uri).await?;

        let verifier = Self {
            issuer_url: trimmed.clone(),
            audience: audience.into_iter().collect(),
            jwks_uri: jwks_uri.clone(),
            jwks: Arc::new(RwLock::new(jwks)),
            refresh_interval: Duration::from_secs(refresh_secs.max(60)),
        };

        verifier.spawn_refresh(client);
        Ok(verifier)
    }

    /// `iss` claim this verifier accepts. Useful for matching incoming
    /// tokens to the right verifier without calling [`Self::verify`]
    /// (avoids signature work for tokens from other issuers).
    pub fn issuer(&self) -> &str {
        &self.issuer_url
    }

    /// Verify a JWT. The token's `iss` must match this verifier's
    /// configured issuer; `aud` must overlap the configured audience
    /// (or audience must be empty — operator opt-in to skip aud check).
    /// Signature is verified against the cached JWKS, looked up by
    /// `kid` from the JWT header.
    pub fn verify<T: DeserializeOwned>(&self, token: &str) -> Result<jsonwebtoken::TokenData<T>> {
        let header =
            decode_header(token).map_err(|e| Error::Oidc(format!("decode jwt header: {e}")))?;
        let kid = header
            .kid
            .as_ref()
            .ok_or_else(|| Error::Oidc("jwt header missing `kid`".to_string()))?;

        let jwk = {
            let jwks = self.jwks.read();
            jwks.find(kid).cloned()
        };
        let jwk = jwk.ok_or_else(|| {
            Error::Oidc(format!(
                "kid `{kid}` not in cached jwks for issuer `{}`",
                self.issuer_url
            ))
        })?;

        let key = DecodingKey::from_jwk(&jwk)
            .map_err(|e| Error::Oidc(format!("build decoding key from jwk: {e}")))?;

        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[&self.issuer_url]);
        if self.audience.is_empty() {
            // Operator explicitly opted out of audience checking.
            // jsonwebtoken's default validation requires `aud`, so disable it.
            validation.validate_aud = false;
        } else {
            let aud: Vec<&str> = self.audience.iter().map(String::as_str).collect();
            validation.set_audience(&aud);
        }

        decode::<T>(token, &key, &validation)
            .map_err(|e| Error::Oidc(format!("verify jwt against `{}`: {e}", self.issuer_url)))
    }

    fn spawn_refresh(&self, client: reqwest::Client) {
        let jwks = Arc::clone(&self.jwks);
        let jwks_uri = self.jwks_uri.clone();
        let interval = self.refresh_interval;
        let issuer_url = self.issuer_url.clone();

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                match fetch_jwks(&client, &jwks_uri).await {
                    Ok(fresh) => {
                        *jwks.write() = fresh;
                        tracing::debug!(
                            target: "assay-auth::external_jwt",
                            issuer = %issuer_url,
                            "refreshed jwks"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "assay-auth::external_jwt",
                            issuer = %issuer_url,
                            error = %e,
                            "failed to refresh jwks; keeping previous keys"
                        );
                    }
                }
            }
        });
    }
}

async fn fetch_jwks(client: &reqwest::Client, uri: &str) -> Result<JwkSet> {
    let body: serde_json::Value = client
        .get(uri)
        .send()
        .await
        .map_err(|e| Error::Oidc(format!("fetch jwks {uri}: {e}")))?
        .error_for_status()
        .map_err(|e| Error::Oidc(format!("fetch jwks {uri}: {e}")))?
        .json()
        .await
        .map_err(|e| Error::Oidc(format!("parse jwks {uri}: {e}")))?;
    serde_json::from_value(body).map_err(|e| Error::Oidc(format!("decode jwks {uri}: {e}")))
}

/// Look up the verifier matching the token's `iss` claim and validate
/// against it. Decodes the token's claims twice — once unverified to
/// pull out `iss`, once verified — but the unverified decode is just a
/// base64 split, so the extra cost is negligible. Returns `None` if no
/// configured verifier accepts the token's `iss` (caller falls through
/// to the next auth strategy).
pub fn verify_with_any<T: DeserializeOwned>(
    issuers: &[ExternalJwtIssuer],
    token: &str,
) -> Option<Result<jsonwebtoken::TokenData<T>>> {
    if issuers.is_empty() {
        return None;
    }

    // Unverified peek at `iss` so we route directly to the right
    // verifier instead of trying every key set linearly. The actual
    // signature + claim verification happens inside the matched
    // verifier — `insecure_decode` here only parses the payload.
    #[derive(serde::Deserialize)]
    struct IssClaim {
        iss: String,
    }
    let unverified = insecure_decode::<IssClaim>(token).ok()?;
    let iss = unverified.claims.iss;
    let trimmed = iss.trim_end_matches('/');

    for issuer in issuers {
        if issuer.issuer() == trimmed || issuer.issuer() == iss {
            return Some(issuer.verify::<T>(token));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct TestClaims {
        iss: String,
        aud: String,
        sub: String,
        exp: usize,
    }

    /// Build a verifier with a hand-crafted JWKS — skips the discovery
    /// network call so unit tests stay hermetic. Uses HS256 because
    /// jsonwebtoken's `from_jwk` accepts symmetric keys, and we don't
    /// need RSA's complexity for proving the verifier wires up.
    fn verifier_for_tests(issuer: &str, audience: Vec<String>, jwks: JwkSet) -> ExternalJwtIssuer {
        ExternalJwtIssuer {
            issuer_url: issuer.trim_end_matches('/').to_string(),
            audience: audience.into_iter().collect(),
            jwks_uri: format!("{issuer}/jwks"),
            jwks: Arc::new(RwLock::new(jwks)),
            refresh_interval: Duration::from_secs(3600),
        }
    }

    fn hs256_jwks_with_kid(kid: &str, secret: &[u8]) -> JwkSet {
        let json = serde_json::json!({
            "keys": [{
                "kty": "oct",
                "use": "sig",
                "alg": "HS256",
                "kid": kid,
                "k": base64_url(secret)
            }]
        });
        serde_json::from_value(json).unwrap()
    }

    fn base64_url(b: &[u8]) -> String {
        // Hand-rolled base64url (RFC 4648 §5, no padding) so the test
        // doesn't pull in an extra dev-dep just to encode a symmetric
        // key into a test JWK.
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = Vec::with_capacity(b.len().div_ceil(3) * 4);
        for chunk in b.chunks(3) {
            let mut buf = [0u8; 3];
            buf[..chunk.len()].copy_from_slice(chunk);
            let n = u32::from_be_bytes([0, buf[0], buf[1], buf[2]]);
            out.push(T[((n >> 18) & 0x3F) as usize]);
            out.push(T[((n >> 12) & 0x3F) as usize]);
            if chunk.len() >= 2 {
                out.push(T[((n >> 6) & 0x3F) as usize]);
            }
            if chunk.len() == 3 {
                out.push(T[(n & 0x3F) as usize]);
            }
        }
        String::from_utf8(out).expect("ascii")
    }

    fn issue_test_token(secret: &[u8], kid: &str, claims: &TestClaims) -> String {
        let mut header = Header::new(Algorithm::HS256);
        header.kid = Some(kid.to_string());
        encode(&header, claims, &EncodingKey::from_secret(secret)).unwrap()
    }

    #[test]
    fn verifies_token_from_configured_issuer() {
        let secret = b"unit-test-secret-key-32bytes!!!!";
        let kid = "test-key-1";
        let issuer = "https://hydra.example.com";
        let aud = "test-app";
        let claims = TestClaims {
            iss: issuer.to_string(),
            aud: aud.to_string(),
            sub: "user-42".to_string(),
            exp: (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600) as usize,
        };
        let token = issue_test_token(secret, kid, &claims);

        let v = verifier_for_tests(
            issuer,
            vec![aud.to_string()],
            hs256_jwks_with_kid(kid, secret),
        );
        let out = v.verify::<TestClaims>(&token).unwrap();
        assert_eq!(out.claims, claims);
    }

    #[test]
    fn rejects_token_with_wrong_issuer() {
        let secret = b"unit-test-secret-key-32bytes!!!!";
        let kid = "test-key-1";
        let claims = TestClaims {
            iss: "https://other.example.com".to_string(),
            aud: "test-app".to_string(),
            sub: "user-42".to_string(),
            exp: (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600) as usize,
        };
        let token = issue_test_token(secret, kid, &claims);

        let v = verifier_for_tests(
            "https://hydra.example.com",
            vec!["test-app".to_string()],
            hs256_jwks_with_kid(kid, secret),
        );
        assert!(v.verify::<TestClaims>(&token).is_err());
    }

    #[test]
    fn rejects_token_with_wrong_audience() {
        let secret = b"unit-test-secret-key-32bytes!!!!";
        let kid = "test-key-1";
        let issuer = "https://hydra.example.com";
        let claims = TestClaims {
            iss: issuer.to_string(),
            aud: "some-other-app".to_string(),
            sub: "user-42".to_string(),
            exp: (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600) as usize,
        };
        let token = issue_test_token(secret, kid, &claims);

        let v = verifier_for_tests(
            issuer,
            vec!["test-app".to_string()],
            hs256_jwks_with_kid(kid, secret),
        );
        assert!(v.verify::<TestClaims>(&token).is_err());
    }

    #[test]
    fn rejects_token_with_unknown_kid() {
        let secret = b"unit-test-secret-key-32bytes!!!!";
        let issuer = "https://hydra.example.com";
        let claims = TestClaims {
            iss: issuer.to_string(),
            aud: "test-app".to_string(),
            sub: "user-42".to_string(),
            exp: (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600) as usize,
        };
        // Token signed with kid="rotated-key" but JWKS only knows "current-key".
        let token = issue_test_token(secret, "rotated-key", &claims);

        let v = verifier_for_tests(
            issuer,
            vec!["test-app".to_string()],
            hs256_jwks_with_kid("current-key", secret),
        );
        let err = v.verify::<TestClaims>(&token).unwrap_err().to_string();
        assert!(err.contains("kid"), "error should mention kid: {err}");
    }

    #[test]
    fn rejects_expired_token() {
        let secret = b"unit-test-secret-key-32bytes!!!!";
        let kid = "test-key-1";
        let issuer = "https://hydra.example.com";
        let claims = TestClaims {
            iss: issuer.to_string(),
            aud: "test-app".to_string(),
            sub: "user-42".to_string(),
            // 1 hour ago — already expired.
            exp: (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                - 3600) as usize,
        };
        let token = issue_test_token(secret, kid, &claims);

        let v = verifier_for_tests(
            issuer,
            vec!["test-app".to_string()],
            hs256_jwks_with_kid(kid, secret),
        );
        assert!(v.verify::<TestClaims>(&token).is_err());
    }

    #[test]
    fn empty_audience_list_skips_aud_check() {
        let secret = b"unit-test-secret-key-32bytes!!!!";
        let kid = "test-key-1";
        let issuer = "https://hydra.example.com";
        let claims = TestClaims {
            iss: issuer.to_string(),
            aud: "literally-anything".to_string(),
            sub: "user-42".to_string(),
            exp: (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600) as usize,
        };
        let token = issue_test_token(secret, kid, &claims);

        // No audience configured → operator opted out of `aud` checking.
        let v = verifier_for_tests(issuer, vec![], hs256_jwks_with_kid(kid, secret));
        assert!(v.verify::<TestClaims>(&token).is_ok());
    }

    #[test]
    fn verify_with_any_routes_by_iss() {
        let secret_a = b"key-A-secret-32bytes-unit-tests!";
        let secret_b = b"key-B-secret-32bytes-unit-tests!";
        let issuer_a = "https://hydra-a.example.com";
        let issuer_b = "https://hydra-b.example.com";

        let v_a = verifier_for_tests(
            issuer_a,
            vec!["test-app".to_string()],
            hs256_jwks_with_kid("a-key", secret_a),
        );
        let v_b = verifier_for_tests(
            issuer_b,
            vec!["test-app".to_string()],
            hs256_jwks_with_kid("b-key", secret_b),
        );

        let claims_b = TestClaims {
            iss: issuer_b.to_string(),
            aud: "test-app".to_string(),
            sub: "user-42".to_string(),
            exp: (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600) as usize,
        };
        let token_b = issue_test_token(secret_b, "b-key", &claims_b);

        // Token from issuer B should route to verifier B and succeed.
        let result = verify_with_any::<TestClaims>(&[v_a, v_b], &token_b)
            .expect("verifier should match issuer_b")
            .expect("verification should succeed");
        assert_eq!(result.claims, claims_b);
    }

    #[test]
    fn verify_with_any_returns_none_for_unknown_issuer() {
        let secret = b"unit-test-secret-key-32bytes!!!!";
        let v = verifier_for_tests(
            "https://hydra.example.com",
            vec!["test-app".to_string()],
            hs256_jwks_with_kid("a-key", secret),
        );
        let claims = TestClaims {
            iss: "https://stranger.example.com".to_string(),
            aud: "test-app".to_string(),
            sub: "user-42".to_string(),
            exp: (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600) as usize,
        };
        let token = issue_test_token(secret, "a-key", &claims);

        let result = verify_with_any::<TestClaims>(&[v], &token);
        assert!(result.is_none(), "unknown issuer should fall through");
    }

    #[test]
    fn verify_with_any_returns_none_for_empty_issuer_list() {
        let secret = b"unit-test-secret-key-32bytes!!!!";
        let claims = TestClaims {
            iss: "https://anywhere.example.com".to_string(),
            aud: "test-app".to_string(),
            sub: "user-42".to_string(),
            exp: 9999999999,
        };
        let token = issue_test_token(secret, "x", &claims);
        assert!(verify_with_any::<TestClaims>(&[], &token).is_none());
    }
}
