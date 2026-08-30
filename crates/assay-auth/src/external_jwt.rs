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
use jsonwebtoken::jwk::{
    AlgorithmParameters, EllipticCurve, Jwk, JwkSet, KeyAlgorithm,
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
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

        // SECURITY: the set of acceptable signature algorithms comes from
        // the *server-side* JWK we matched on `kid`, never from the
        // attacker-controlled `header.alg`. This blocks alg-confusion
        // (e.g. an RSA verification key forced to verify an HS256 token,
        // where the attacker signs with the RSA public key as the HMAC
        // secret) and `alg: none`. Reject up front if the token's header
        // alg is not one this key can legitimately produce.
        let allowed = allowed_algorithms_for_jwk(&jwk)?;
        if !allowed.contains(&header.alg) {
            return Err(Error::Oidc(format!(
                "jwt header alg {:?} not permitted for kid `{kid}` (issuer `{}`); \
                 allowed: {allowed:?}",
                header.alg, self.issuer_url
            )));
        }

        let key = DecodingKey::from_jwk(&jwk)
            .map_err(|e| Error::Oidc(format!("build decoding key from jwk: {e}")))?;

        // Pin the verifier to the server-derived algorithm set. `decode`
        // independently re-checks `header.alg ∈ validation.algorithms`,
        // so this is the load-bearing control even if the explicit guard
        // above is ever refactored away.
        let mut validation = Validation::new(allowed[0]);
        validation.algorithms = allowed;
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

/// Derive the set of signature algorithms a given JWK is allowed to
/// verify, computed purely from server-side key material — the JWK's
/// declared `alg` (`common.key_algorithm`) when present, otherwise its
/// key type / curve (`kty`/`crv`). The inbound token header is never
/// consulted here.
///
/// External issuers are asymmetric-only: symmetric (HMAC / `oct`) keys
/// are hard-rejected. A JWKS published at a public `jwks_uri` should
/// only ever expose public asymmetric keys; an `oct` entry would mean
/// the verification secret is also the signing secret, which both
/// defeats pass-through trust and is the classic RS256→HS256
/// confusion primitive. `alg: none` can never appear in the returned
/// set, so unsigned tokens are rejected.
fn allowed_algorithms_for_jwk(jwk: &Jwk) -> Result<Vec<Algorithm>> {
    // If the operator/IdP pinned `alg` on the key, honor exactly that
    // one algorithm — but still refuse symmetric algorithms outright.
    if let Some(key_alg) = jwk.common.key_algorithm {
        if matches!(
            key_alg,
            KeyAlgorithm::HS256 | KeyAlgorithm::HS384 | KeyAlgorithm::HS512
        ) {
            return Err(Error::Oidc(
                "external issuer JWK declares a symmetric (HS*) alg; \
                 only asymmetric keys are accepted"
                    .to_string(),
            ));
        }
        let alg = match key_alg {
            KeyAlgorithm::ES256 => Algorithm::ES256,
            KeyAlgorithm::ES384 => Algorithm::ES384,
            KeyAlgorithm::RS256 => Algorithm::RS256,
            KeyAlgorithm::RS384 => Algorithm::RS384,
            KeyAlgorithm::RS512 => Algorithm::RS512,
            KeyAlgorithm::PS256 => Algorithm::PS256,
            KeyAlgorithm::PS384 => Algorithm::PS384,
            KeyAlgorithm::PS512 => Algorithm::PS512,
            KeyAlgorithm::EdDSA => Algorithm::EdDSA,
            // HS* handled above; RSA-encryption algs (RSA1_5 / RSA-OAEP*)
            // and UNKNOWN_ALGORITHM are not JWS signature algorithms.
            other => {
                return Err(Error::Oidc(format!(
                    "external issuer JWK declares unsupported signature alg {other:?}"
                )));
            }
        };
        return Ok(vec![alg]);
    }

    // No `alg` on the key — derive the acceptable set from the key type.
    match &jwk.algorithm {
        AlgorithmParameters::OctetKey(_) => Err(Error::Oidc(
            "external issuer JWK is a symmetric (oct) key; only asymmetric keys are accepted"
                .to_string(),
        )),
        AlgorithmParameters::RSA(_) => {
            // RSA keys verify any of the RSASSA-PKCS1 / RSASSA-PSS
            // signature algorithms; the signature math binds the
            // concrete one, so accepting the family is safe.
            Ok(vec![
                Algorithm::RS256,
                Algorithm::RS384,
                Algorithm::RS512,
                Algorithm::PS256,
                Algorithm::PS384,
                Algorithm::PS512,
            ])
        }
        AlgorithmParameters::EllipticCurve(ec) => match &ec.curve {
            EllipticCurve::P256 => Ok(vec![Algorithm::ES256]),
            EllipticCurve::P384 => Ok(vec![Algorithm::ES384]),
            other => Err(Error::Oidc(format!(
                "external issuer EC JWK uses unsupported curve {other:?}"
            ))),
        },
        AlgorithmParameters::OctetKeyPair(okp) => match &okp.curve {
            EllipticCurve::Ed25519 => Ok(vec![Algorithm::EdDSA]),
            other => Err(Error::Oidc(format!(
                "external issuer OKP JWK uses unsupported curve {other:?}"
            ))),
        },
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
    use data_encoding::BASE64URL_NOPAD;
    use ed25519_dalek::SigningKey;
    use ed25519_dalek::pkcs8::EncodePrivateKey;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct TestClaims {
        iss: String,
        aud: String,
        sub: String,
        exp: usize,
    }

    /// A hermetic Ed25519 signer + its published public JWK. External
    /// issuers are asymmetric-only, so tests mint a real EdDSA keypair
    /// (no discovery network call), publish the OKP *public* JWK in the
    /// verifier's JWKS, and sign tokens with the private half.
    struct TestSigner {
        signing_pem: String,
        public_jwk: serde_json::Value,
    }

    fn test_signer(kid: &str) -> TestSigner {
        let signing = SigningKey::generate(&mut rand_core_06::OsRng);
        let signing_pem = signing
            .to_pkcs8_pem(ed25519_dalek::pkcs8::spki::der::pem::LineEnding::LF)
            .expect("ed25519 PKCS#8 PEM")
            .to_string();
        let pub_bytes = signing.verifying_key().to_bytes();
        let public_jwk = serde_json::json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "use": "sig",
            "alg": "EdDSA",
            "kid": kid,
            "x": BASE64URL_NOPAD.encode(&pub_bytes),
        });
        TestSigner {
            signing_pem,
            public_jwk,
        }
    }

    /// Build a verifier with a hand-crafted JWKS — skips the discovery
    /// network call so unit tests stay hermetic.
    fn verifier_for_tests(issuer: &str, audience: Vec<String>, jwks: JwkSet) -> ExternalJwtIssuer {
        ExternalJwtIssuer {
            issuer_url: issuer.trim_end_matches('/').to_string(),
            audience: audience.into_iter().collect(),
            jwks_uri: format!("{issuer}/jwks"),
            jwks: Arc::new(RwLock::new(jwks)),
            refresh_interval: Duration::from_secs(3600),
        }
    }

    /// JWKS containing only `signer`'s public key.
    fn jwks_from_signer(signer: &TestSigner) -> JwkSet {
        let json = serde_json::json!({ "keys": [signer.public_jwk] });
        serde_json::from_value(json).unwrap()
    }

    /// Sign claims with the signer's Ed25519 private key (correct EdDSA
    /// path). `kid` is stamped into the header.
    fn issue_test_token(signer: &TestSigner, kid: &str, claims: &TestClaims) -> String {
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(kid.to_string());
        let key = EncodingKey::from_ed_pem(signer.signing_pem.as_bytes()).unwrap();
        encode(&header, claims, &key).unwrap()
    }

    fn future_exp() -> usize {
        (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600) as usize
    }

    #[test]
    fn verifies_token_from_configured_issuer() {
        let kid = "test-key-1";
        let issuer = "https://hydra.example.com";
        let aud = "test-app";
        let signer = test_signer(kid);
        let claims = TestClaims {
            iss: issuer.to_string(),
            aud: aud.to_string(),
            sub: "user-42".to_string(),
            exp: future_exp(),
        };
        let token = issue_test_token(&signer, kid, &claims);

        let v = verifier_for_tests(issuer, vec![aud.to_string()], jwks_from_signer(&signer));
        let out = v.verify::<TestClaims>(&token).unwrap();
        assert_eq!(out.claims, claims);
    }

    #[test]
    fn rejects_token_with_wrong_issuer() {
        let kid = "test-key-1";
        let signer = test_signer(kid);
        let claims = TestClaims {
            iss: "https://other.example.com".to_string(),
            aud: "test-app".to_string(),
            sub: "user-42".to_string(),
            exp: future_exp(),
        };
        let token = issue_test_token(&signer, kid, &claims);

        let v = verifier_for_tests(
            "https://hydra.example.com",
            vec!["test-app".to_string()],
            jwks_from_signer(&signer),
        );
        assert!(v.verify::<TestClaims>(&token).is_err());
    }

    #[test]
    fn rejects_token_with_wrong_audience() {
        let kid = "test-key-1";
        let issuer = "https://hydra.example.com";
        let signer = test_signer(kid);
        let claims = TestClaims {
            iss: issuer.to_string(),
            aud: "some-other-app".to_string(),
            sub: "user-42".to_string(),
            exp: future_exp(),
        };
        let token = issue_test_token(&signer, kid, &claims);

        let v = verifier_for_tests(
            issuer,
            vec!["test-app".to_string()],
            jwks_from_signer(&signer),
        );
        assert!(v.verify::<TestClaims>(&token).is_err());
    }

    #[test]
    fn rejects_token_with_unknown_kid() {
        let issuer = "https://hydra.example.com";
        let signer = test_signer("rotated-key");
        let claims = TestClaims {
            iss: issuer.to_string(),
            aud: "test-app".to_string(),
            sub: "user-42".to_string(),
            exp: future_exp(),
        };
        // Token signed with kid="rotated-key" but JWKS only knows "current-key".
        let token = issue_test_token(&signer, "rotated-key", &claims);

        let current = test_signer("current-key");
        let v = verifier_for_tests(
            issuer,
            vec!["test-app".to_string()],
            jwks_from_signer(&current),
        );
        let err = v.verify::<TestClaims>(&token).unwrap_err().to_string();
        assert!(err.contains("kid"), "error should mention kid: {err}");
    }

    #[test]
    fn rejects_expired_token() {
        let kid = "test-key-1";
        let issuer = "https://hydra.example.com";
        let signer = test_signer(kid);
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
        let token = issue_test_token(&signer, kid, &claims);

        let v = verifier_for_tests(
            issuer,
            vec!["test-app".to_string()],
            jwks_from_signer(&signer),
        );
        assert!(v.verify::<TestClaims>(&token).is_err());
    }

    #[test]
    fn empty_audience_list_skips_aud_check() {
        let kid = "test-key-1";
        let issuer = "https://hydra.example.com";
        let signer = test_signer(kid);
        let claims = TestClaims {
            iss: issuer.to_string(),
            aud: "literally-anything".to_string(),
            sub: "user-42".to_string(),
            exp: future_exp(),
        };
        let token = issue_test_token(&signer, kid, &claims);

        // No audience configured → operator opted out of `aud` checking.
        let v = verifier_for_tests(issuer, vec![], jwks_from_signer(&signer));
        assert!(v.verify::<TestClaims>(&token).is_ok());
    }

    #[test]
    fn verify_with_any_routes_by_iss() {
        let issuer_a = "https://hydra-a.example.com";
        let issuer_b = "https://hydra-b.example.com";
        let signer_a = test_signer("a-key");
        let signer_b = test_signer("b-key");

        let v_a = verifier_for_tests(
            issuer_a,
            vec!["test-app".to_string()],
            jwks_from_signer(&signer_a),
        );
        let v_b = verifier_for_tests(
            issuer_b,
            vec!["test-app".to_string()],
            jwks_from_signer(&signer_b),
        );

        let claims_b = TestClaims {
            iss: issuer_b.to_string(),
            aud: "test-app".to_string(),
            sub: "user-42".to_string(),
            exp: future_exp(),
        };
        let token_b = issue_test_token(&signer_b, "b-key", &claims_b);

        // Token from issuer B should route to verifier B and succeed.
        let result = verify_with_any::<TestClaims>(&[v_a, v_b], &token_b)
            .expect("verifier should match issuer_b")
            .expect("verification should succeed");
        assert_eq!(result.claims, claims_b);
    }

    #[test]
    fn verify_with_any_returns_none_for_unknown_issuer() {
        let signer = test_signer("a-key");
        let v = verifier_for_tests(
            "https://hydra.example.com",
            vec!["test-app".to_string()],
            jwks_from_signer(&signer),
        );
        let claims = TestClaims {
            iss: "https://stranger.example.com".to_string(),
            aud: "test-app".to_string(),
            sub: "user-42".to_string(),
            exp: future_exp(),
        };
        let token = issue_test_token(&signer, "a-key", &claims);

        let result = verify_with_any::<TestClaims>(&[v], &token);
        assert!(result.is_none(), "unknown issuer should fall through");
    }

    #[test]
    fn verify_with_any_returns_none_for_empty_issuer_list() {
        let signer = test_signer("x");
        let claims = TestClaims {
            iss: "https://anywhere.example.com".to_string(),
            aud: "test-app".to_string(),
            sub: "user-42".to_string(),
            exp: 9999999999,
        };
        let token = issue_test_token(&signer, "x", &claims);
        assert!(verify_with_any::<TestClaims>(&[], &token).is_none());
    }

    // ----- Algorithm-pinning regression tests (HIGH-severity finding) -----

    /// Build a JWKS whose single key is the Ed25519 *public* key from
    /// `signer` but with its declared `alg`/`kty` overridden to the
    /// supplied JSON — lets us forge a JWKS that *looks* like a symmetric
    /// key while still carrying real Ed25519 material.
    fn jwks_with_overridden_key(signer: &TestSigner, overrides: serde_json::Value) -> JwkSet {
        let mut key = signer.public_jwk.clone();
        if let (Some(obj), Some(ov)) = (key.as_object_mut(), overrides.as_object()) {
            for (k, val) in ov {
                obj.insert(k.clone(), val.clone());
            }
        }
        serde_json::from_value(serde_json::json!({ "keys": [key] })).unwrap()
    }

    /// (a) A token whose header advertises an unexpected-but-valid-looking
    /// alg (RS256) must be rejected even though it is correctly EdDSA-signed
    /// and the kid matches — the alg is pinned from the server-side JWK, not
    /// the header.
    #[test]
    fn rejects_header_alg_mismatch_against_key() {
        let kid = "test-key-1";
        let issuer = "https://hydra.example.com";
        let signer = test_signer(kid);
        let claims = TestClaims {
            iss: issuer.to_string(),
            aud: "test-app".to_string(),
            sub: "user-42".to_string(),
            exp: future_exp(),
        };

        // Correctly EdDSA-sign, then rewrite the header to claim RS256.
        let real = issue_test_token(&signer, kid, &claims);
        let mut parts = real.split('.');
        let _real_header = parts.next().unwrap();
        let payload = parts.next().unwrap();
        let sig = parts.next().unwrap();
        let forged_header = BASE64URL_NOPAD
            .encode(br#"{"alg":"RS256","typ":"JWT","kid":"test-key-1"}"#);
        let forged = format!("{forged_header}.{payload}.{sig}");

        let v = verifier_for_tests(
            issuer,
            vec!["test-app".to_string()],
            jwks_from_signer(&signer),
        );
        let err = v.verify::<TestClaims>(&forged).unwrap_err().to_string();
        assert!(
            err.contains("not permitted") || err.contains("RS256") || err.contains("InvalidAlgorithm"),
            "expected alg-mismatch rejection, got: {err}"
        );
    }

    /// (b) Alg-confusion: an issuer that publishes only an asymmetric
    /// (Ed25519) key must never let an attacker authenticate with an HS*
    /// token. Even if the JWKS entry is tampered to advertise a symmetric
    /// alg, the verifier hard-rejects symmetric keys for external issuers.
    #[test]
    fn rejects_alg_confusion_symmetric_key() {
        let kid = "test-key-1";
        let issuer = "https://hydra.example.com";
        let signer = test_signer(kid);
        let claims = TestClaims {
            iss: issuer.to_string(),
            aud: "test-app".to_string(),
            sub: "user-42".to_string(),
            exp: future_exp(),
        };

        // Attacker crafts an HS256 token, signing with the issuer's public
        // key bytes as the HMAC secret (the classic RS/ES→HS confusion).
        let pub_bytes = {
            let pk = signer.public_jwk["x"].as_str().unwrap();
            BASE64URL_NOPAD.decode(pk.as_bytes()).unwrap()
        };
        let mut header = Header::new(Algorithm::HS256);
        header.kid = Some(kid.to_string());
        let forged = encode(
            &header,
            &claims,
            &EncodingKey::from_secret(&pub_bytes),
        )
        .unwrap();

        // JWKS still publishes the legitimate asymmetric OKP key.
        let v = verifier_for_tests(
            issuer,
            vec!["test-app".to_string()],
            jwks_from_signer(&signer),
        );
        assert!(
            v.verify::<TestClaims>(&forged).is_err(),
            "HS256 token forged with the public key must be rejected"
        );

        // And even a tampered JWKS that *declares* a symmetric alg is
        // refused outright before any signature check.
        let sym_jwks = jwks_with_overridden_key(
            &signer,
            serde_json::json!({ "kty": "oct", "alg": "HS256", "k": "AAAA" }),
        );
        let v2 = verifier_for_tests(issuer, vec!["test-app".to_string()], sym_jwks);
        let err = v2.verify::<TestClaims>(&forged).unwrap_err().to_string();
        assert!(
            err.contains("symmetric"),
            "symmetric JWK should be hard-rejected: {err}"
        );
    }

    /// (c) Happy path still verifies a correctly EdDSA-signed token whose
    /// header alg matches the published key.
    #[test]
    fn happy_path_eddsa_still_verifies() {
        let kid = "test-key-1";
        let issuer = "https://hydra.example.com";
        let signer = test_signer(kid);
        let claims = TestClaims {
            iss: issuer.to_string(),
            aud: "test-app".to_string(),
            sub: "user-42".to_string(),
            exp: future_exp(),
        };
        let token = issue_test_token(&signer, kid, &claims);

        let v = verifier_for_tests(
            issuer,
            vec!["test-app".to_string()],
            jwks_from_signer(&signer),
        );
        let out = v.verify::<TestClaims>(&token).unwrap();
        assert_eq!(out.claims, claims);
    }

    /// `alg: none` (unsigned) tokens must be rejected — `none` can never
    /// be in the server-derived allowed set.
    #[test]
    fn rejects_alg_none_token() {
        let kid = "test-key-1";
        let issuer = "https://hydra.example.com";
        let signer = test_signer(kid);
        let claims = TestClaims {
            iss: issuer.to_string(),
            aud: "test-app".to_string(),
            sub: "user-42".to_string(),
            exp: future_exp(),
        };
        let header = BASE64URL_NOPAD.encode(br#"{"alg":"none","typ":"JWT","kid":"test-key-1"}"#);
        let payload = BASE64URL_NOPAD.encode(serde_json::to_vec(&claims).unwrap().as_slice());
        // Unsigned: empty signature segment.
        let unsigned = format!("{header}.{payload}.");

        let v = verifier_for_tests(
            issuer,
            vec!["test-app".to_string()],
            jwks_from_signer(&signer),
        );
        assert!(
            v.verify::<TestClaims>(&unsigned).is_err(),
            "alg:none token must be rejected"
        );
    }
}
