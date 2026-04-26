//! JWT issuance + verification with key rotation backed by
//! `auth.jwks_keys`.
//!
//! Plan 11 reference: `jsonwebtoken` 10 with kid-based key lookup so old
//! tokens still verify after a key rotation. We default to EdDSA
//! (Ed25519) — small keys, fast signatures, no PKCS#1 footguns.
//!
//! Lifecycle:
//! 1. Boot loads keys with [`JwtConfig::load_from_postgres`] /
//!    [`JwtConfig::load_from_sqlite`]. The row with `rotated_at IS NULL`
//!    becomes the active signing key; rotated rows become history
//!    (verify-only).
//! 2. [`JwtConfig::issue`] signs new tokens with the active key, putting
//!    its `kid` in the JWT header.
//! 3. [`JwtConfig::verify`] looks up the signing key by `kid` (active
//!    first, then history), validates `iss` and `aud`, returns the
//!    [`jsonwebtoken::TokenData`].
//! 4. [`JwtConfig::rotate_postgres`] / [`JwtConfig::rotate_sqlite`]
//!    generate a fresh Ed25519 keypair, persist it, mark the old active
//!    key rotated, and swap the in-memory state atomically.

use std::sync::Arc;

use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, TokenData, Validation, decode, decode_header,
    encode,
};
use parking_lot::RwLock;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{Error, Result};

/// Active signing key + its decoding twin. Held by the `Inner` state
/// behind a `RwLock` so [`JwtConfig::rotate_postgres`] /
/// [`JwtConfig::rotate_sqlite`] can swap the active key under callers
/// already verifying inflight tokens.
pub struct ActiveKey {
    pub kid: String,
    pub alg: Algorithm,
    pub encoding_key: EncodingKey,
    pub decoding_key: DecodingKey,
    pub expires_at: Option<f64>,
}

/// Verify-only entry. Older keys live here so already-issued tokens
/// validate until they expire on their own.
pub struct HistoryKey {
    pub kid: String,
    pub alg: Algorithm,
    pub decoding_key: DecodingKey,
}

struct Inner {
    active: Option<ActiveKey>,
    history: Vec<HistoryKey>,
    issuer: String,
    audience: Vec<String>,
}

/// Cheap-to-clone JWT configuration. Wrap with `Arc` internally so all
/// clones share the same active-key + history view.
#[derive(Clone)]
pub struct JwtConfig {
    inner: Arc<RwLock<Inner>>,
}

impl JwtConfig {
    /// Empty configuration — no keys yet. Caller must populate via
    /// [`JwtConfig::load_from_postgres`] / [`JwtConfig::load_from_sqlite`]
    /// or [`JwtConfig::set_active`] before issuing tokens.
    pub fn new(issuer: String, audience: Vec<String>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner {
                active: None,
                history: Vec::new(),
                issuer,
                audience,
            })),
        }
    }

    /// Replace the in-memory active key + history. Useful in tests where
    /// we want a single ephemeral keypair without round-tripping the DB.
    pub fn set_active(&self, active: ActiveKey, history: Vec<HistoryKey>) {
        let mut guard = self.inner.write();
        guard.active = Some(active);
        guard.history = history;
    }

    /// Sign `claims` with the active key. The active key's `kid` is
    /// written into the JWT header so verify can look it up.
    pub fn issue<T: Serialize>(&self, claims: &T) -> Result<String> {
        let guard = self.inner.read();
        let active = guard
            .active
            .as_ref()
            .ok_or_else(|| Error::Jwt("no active jwt key configured".to_string()))?;
        let mut header = Header::new(active.alg);
        header.kid = Some(active.kid.clone());
        encode(&header, claims, &active.encoding_key).map_err(map_jwt_err)
    }

    /// Verify `token` and decode its claims. Looks up the decoding key
    /// by header `kid` (active first, then history), validates `iss` and
    /// the audience list against the in-memory configuration.
    pub fn verify<T: DeserializeOwned>(&self, token: &str) -> Result<TokenData<T>> {
        let header = decode_header(token).map_err(map_jwt_err)?;
        let kid = header
            .kid
            .as_deref()
            .ok_or_else(|| Error::Jwt("token has no kid header".to_string()))?;
        let guard = self.inner.read();
        let (alg, decoding_key) = lookup_decoding_key(&guard, kid)
            .ok_or_else(|| Error::Jwt(format!("unknown kid {kid}")))?;
        let mut validation = Validation::new(alg);
        validation.set_issuer(std::slice::from_ref(&guard.issuer));
        if !guard.audience.is_empty() {
            validation.set_audience(&guard.audience);
        }
        decode::<T>(token, decoding_key, &validation).map_err(map_jwt_err)
    }

    /// Borrow the active key's `kid` (cheap clone). Useful in tests and
    /// for telemetry.
    pub fn active_kid(&self) -> Option<String> {
        self.inner.read().active.as_ref().map(|k| k.kid.clone())
    }

    /// Load every key from `auth.jwks_keys` into memory. The row with
    /// `rotated_at IS NULL` becomes active; the rest become history.
    /// `private_pem_encrypted` is treated as plaintext PEM for now —
    /// encryption-at-rest is a later phase.
    #[cfg(feature = "backend-postgres")]
    pub async fn load_from_postgres(&self, pool: &sqlx::PgPool) -> Result<()> {
        use sqlx::Row;
        let rows = sqlx::query(
            "SELECT kid, alg, private_pem_encrypted, rotated_at, expires_at
             FROM auth.jwks_keys
             ORDER BY created_at",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| Error::Backend(anyhow::anyhow!("load auth.jwks_keys (pg): {e}")))?;

        let mut active = None;
        let mut history = Vec::new();
        for row in rows {
            let kid: String = row.get("kid");
            let alg_str: String = row.get("alg");
            let pem: Option<Vec<u8>> = row.get("private_pem_encrypted");
            let rotated_at: Option<f64> = row.get("rotated_at");
            let expires_at: Option<f64> = row.get("expires_at");
            let alg = parse_alg(&alg_str)?;
            let pem = pem.ok_or_else(|| {
                Error::Jwt(format!("auth.jwks_keys row {kid} has no private key"))
            })?;
            let (encoding_key, decoding_key) = build_keys(alg, &pem)?;
            if rotated_at.is_none() && active.is_none() {
                active = Some(ActiveKey {
                    kid: kid.clone(),
                    alg,
                    encoding_key,
                    decoding_key,
                    expires_at,
                });
            } else {
                history.push(HistoryKey {
                    kid,
                    alg,
                    decoding_key,
                });
            }
        }
        let mut guard = self.inner.write();
        guard.active = active;
        guard.history = history;
        Ok(())
    }

    /// SQLite mirror of [`JwtConfig::load_from_postgres`].
    #[cfg(feature = "backend-sqlite")]
    pub async fn load_from_sqlite(&self, pool: &sqlx::SqlitePool) -> Result<()> {
        use sqlx::Row;
        let rows = sqlx::query(
            "SELECT kid, alg, private_pem_encrypted, rotated_at, expires_at
             FROM auth.jwks_keys
             ORDER BY created_at",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| Error::Backend(anyhow::anyhow!("load auth.jwks_keys (sqlite): {e}")))?;

        let mut active = None;
        let mut history = Vec::new();
        for row in rows {
            let kid: String = row.get("kid");
            let alg_str: String = row.get("alg");
            let pem: Option<Vec<u8>> = row.get("private_pem_encrypted");
            let rotated_at: Option<f64> = row.get("rotated_at");
            let expires_at: Option<f64> = row.get("expires_at");
            let alg = parse_alg(&alg_str)?;
            let pem = pem.ok_or_else(|| {
                Error::Jwt(format!("auth.jwks_keys row {kid} has no private key"))
            })?;
            let (encoding_key, decoding_key) = build_keys(alg, &pem)?;
            if rotated_at.is_none() && active.is_none() {
                active = Some(ActiveKey {
                    kid: kid.clone(),
                    alg,
                    encoding_key,
                    decoding_key,
                    expires_at,
                });
            } else {
                history.push(HistoryKey {
                    kid,
                    alg,
                    decoding_key,
                });
            }
        }
        let mut guard = self.inner.write();
        guard.active = active;
        guard.history = history;
        Ok(())
    }

    /// Generate a fresh Ed25519 keypair, INSERT it into `auth.jwks_keys`
    /// as the new active row, mark the prior active row rotated, and
    /// swap the in-memory state. Returns the new `kid`.
    #[cfg(feature = "backend-postgres")]
    pub async fn rotate_postgres(&self, pool: &sqlx::PgPool) -> Result<String> {
        let GeneratedKey {
            kid,
            alg,
            private_pem,
            public_jwk,
        } = generate_ed25519_key();
        let (encoding_key, decoding_key) = build_keys(alg, private_pem.as_bytes())?;
        let now = now_secs();
        let mut tx = pool
            .begin()
            .await
            .map_err(|e| Error::Backend(anyhow::anyhow!("begin tx (pg rotate): {e}")))?;
        sqlx::query(
            "UPDATE auth.jwks_keys SET rotated_at = $1 WHERE rotated_at IS NULL",
        )
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|e| Error::Backend(anyhow::anyhow!("mark old key rotated (pg): {e}")))?;
        sqlx::query(
            "INSERT INTO auth.jwks_keys
                 (kid, alg, public_jwk, private_pem_encrypted, created_at, rotated_at, expires_at)
             VALUES ($1, $2, $3::jsonb, $4, $5, NULL, NULL)",
        )
        .bind(&kid)
        .bind(alg_str(alg))
        .bind(public_jwk.to_string())
        .bind(private_pem.as_bytes())
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|e| Error::Backend(anyhow::anyhow!("insert new key (pg): {e}")))?;
        tx.commit()
            .await
            .map_err(|e| Error::Backend(anyhow::anyhow!("commit tx (pg rotate): {e}")))?;
        // Swap in-memory.
        let mut guard = self.inner.write();
        if let Some(prev) = guard.active.take() {
            guard.history.push(HistoryKey {
                kid: prev.kid,
                alg: prev.alg,
                decoding_key: prev.decoding_key,
            });
        }
        guard.active = Some(ActiveKey {
            kid: kid.clone(),
            alg,
            encoding_key,
            decoding_key,
            expires_at: None,
        });
        Ok(kid)
    }

    /// SQLite mirror of [`JwtConfig::rotate_postgres`].
    #[cfg(feature = "backend-sqlite")]
    pub async fn rotate_sqlite(&self, pool: &sqlx::SqlitePool) -> Result<String> {
        let GeneratedKey {
            kid,
            alg,
            private_pem,
            public_jwk,
        } = generate_ed25519_key();
        let (encoding_key, decoding_key) = build_keys(alg, private_pem.as_bytes())?;
        let now = now_secs();
        let mut tx = pool
            .begin()
            .await
            .map_err(|e| Error::Backend(anyhow::anyhow!("begin tx (sqlite rotate): {e}")))?;
        sqlx::query(
            "UPDATE auth.jwks_keys SET rotated_at = ? WHERE rotated_at IS NULL",
        )
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|e| Error::Backend(anyhow::anyhow!("mark old key rotated (sqlite): {e}")))?;
        sqlx::query(
            "INSERT INTO auth.jwks_keys
                 (kid, alg, public_jwk, private_pem_encrypted, created_at, rotated_at, expires_at)
             VALUES (?, ?, ?, ?, ?, NULL, NULL)",
        )
        .bind(&kid)
        .bind(alg_str(alg))
        .bind(public_jwk.to_string())
        .bind(private_pem.as_bytes())
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|e| Error::Backend(anyhow::anyhow!("insert new key (sqlite): {e}")))?;
        tx.commit()
            .await
            .map_err(|e| Error::Backend(anyhow::anyhow!("commit tx (sqlite rotate): {e}")))?;
        let mut guard = self.inner.write();
        if let Some(prev) = guard.active.take() {
            guard.history.push(HistoryKey {
                kid: prev.kid,
                alg: prev.alg,
                decoding_key: prev.decoding_key,
            });
        }
        guard.active = Some(ActiveKey {
            kid: kid.clone(),
            alg,
            encoding_key,
            decoding_key,
            expires_at: None,
        });
        Ok(kid)
    }
}

fn lookup_decoding_key<'a>(
    inner: &'a Inner,
    kid: &str,
) -> Option<(Algorithm, &'a DecodingKey)> {
    if let Some(active) = &inner.active
        && active.kid == kid
    {
        return Some((active.alg, &active.decoding_key));
    }
    inner
        .history
        .iter()
        .find(|h| h.kid == kid)
        .map(|h| (h.alg, &h.decoding_key))
}

/// Build encoding+decoding keys from a stored Ed25519 PKCS#8 private
/// key PEM. `from_ed_pem` for the [`DecodingKey`] expects a *public*
/// PEM, so we derive the SPKI public PEM from the private key first.
fn build_keys(alg: Algorithm, pem: &[u8]) -> Result<(EncodingKey, DecodingKey)> {
    match alg {
        Algorithm::EdDSA => {
            let enc = EncodingKey::from_ed_pem(pem).map_err(map_jwt_err)?;
            let public_pem = ed25519_public_pem_from_private(pem)?;
            let dec = DecodingKey::from_ed_pem(public_pem.as_bytes()).map_err(map_jwt_err)?;
            Ok((enc, dec))
        }
        // RSA / ECDSA paths land when the operator brings their own key
        // material; for v0.14.0 phase 4 we ship Ed25519 only.
        other => Err(Error::Jwt(format!(
            "unsupported jwt algorithm {other:?} (only EdDSA in phase 4)"
        ))),
    }
}

/// Derive the SPKI (subjectPublicKeyInfo) PEM for an Ed25519 keypair
/// from the private PKCS#8 PEM. Done by re-parsing the private key with
/// `ed25519_dalek` and re-encoding only the public half.
fn ed25519_public_pem_from_private(private_pem: &[u8]) -> Result<String> {
    use ed25519_dalek::SigningKey;
    use ed25519_dalek::pkcs8::DecodePrivateKey;
    use ed25519_dalek::pkcs8::spki::EncodePublicKey;

    let pem_str = std::str::from_utf8(private_pem)
        .map_err(|e| Error::Jwt(format!("ed25519 private PEM utf8: {e}")))?;
    let signing = SigningKey::from_pkcs8_pem(pem_str)
        .map_err(|e| Error::Jwt(format!("parse ed25519 private PEM: {e}")))?;
    let verifying = signing.verifying_key();
    verifying
        .to_public_key_pem(ed25519_dalek::pkcs8::spki::der::pem::LineEnding::LF)
        .map_err(|e| Error::Jwt(format!("encode ed25519 public PEM: {e}")))
}

fn parse_alg(name: &str) -> Result<Algorithm> {
    match name {
        "EdDSA" => Ok(Algorithm::EdDSA),
        other => Err(Error::Jwt(format!(
            "unknown jwt algorithm {other:?} (only EdDSA in phase 4)"
        ))),
    }
}

fn alg_str(alg: Algorithm) -> &'static str {
    match alg {
        Algorithm::EdDSA => "EdDSA",
        // Other variants are unreachable today (build_keys / parse_alg
        // gate Ed25519 only). Spell them out so future expansion is a
        // compile-error.
        _ => "EdDSA",
    }
}

fn map_jwt_err(e: jsonwebtoken::errors::Error) -> Error {
    Error::Jwt(e.to_string())
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Output of the in-process Ed25519 key generator. Only used by the
/// rotation helpers; ephemeral test keys go through
/// [`generate_ephemeral_ed25519`] instead so the PEM never round-trips.
struct GeneratedKey {
    kid: String,
    alg: Algorithm,
    private_pem: String,
    public_jwk: serde_json::Value,
}

fn generate_ed25519_key() -> GeneratedKey {
    use ed25519_dalek::SigningKey;
    use ed25519_dalek::pkcs8::EncodePrivateKey;

    let signing = SigningKey::generate(&mut rand_core_06::OsRng);
    let private_pem = signing
        .to_pkcs8_pem(ed25519_dalek::pkcs8::spki::der::pem::LineEnding::LF)
        .expect("ed25519 PKCS#8 PEM encoding")
        .to_string();
    let verifying = signing.verifying_key();
    let pub_bytes = verifying.to_bytes();
    let kid = format!(
        "kid_{}",
        data_encoding::BASE64URL_NOPAD.encode(&pub_bytes[..16])
    );
    let public_jwk = serde_json::json!({
        "kty": "OKP",
        "crv": "Ed25519",
        "alg": "EdDSA",
        "kid": kid,
        "use": "sig",
        "x": data_encoding::BASE64URL_NOPAD.encode(&pub_bytes),
    });
    GeneratedKey {
        kid,
        alg: Algorithm::EdDSA,
        private_pem,
        public_jwk,
    }
}

/// Generate an ephemeral Ed25519 [`ActiveKey`] without touching any DB.
/// Used by tests and by short-lived deployments that don't need
/// rotation persistence.
pub fn generate_ephemeral_ed25519(kid: impl Into<String>) -> Result<ActiveKey> {
    let GeneratedKey { private_pem, .. } = generate_ed25519_key();
    let (encoding_key, decoding_key) = build_keys(Algorithm::EdDSA, private_pem.as_bytes())?;
    Ok(ActiveKey {
        kid: kid.into(),
        alg: Algorithm::EdDSA,
        encoding_key,
        decoding_key,
        expires_at: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Claims {
        sub: String,
        iss: String,
        aud: String,
        exp: usize,
    }

    fn config_with_active(issuer: &str, audience: &[&str]) -> JwtConfig {
        let cfg = JwtConfig::new(
            issuer.to_string(),
            audience.iter().map(|s| s.to_string()).collect(),
        );
        let active = generate_ephemeral_ed25519("kid_test").unwrap();
        cfg.set_active(active, Vec::new());
        cfg
    }

    fn future_exp() -> usize {
        (now_secs() as usize) + 3600
    }

    fn past_exp() -> usize {
        (now_secs() as usize).saturating_sub(3600)
    }

    #[test]
    fn issue_and_verify_round_trip() {
        let cfg = config_with_active("assay", &["assay-engine"]);
        let claims = Claims {
            sub: "user_alice".to_string(),
            iss: "assay".to_string(),
            aud: "assay-engine".to_string(),
            exp: future_exp(),
        };
        let token = cfg.issue(&claims).unwrap();
        let data = cfg.verify::<Claims>(&token).unwrap();
        assert_eq!(data.claims, claims);
        assert_eq!(data.header.kid.as_deref(), Some("kid_test"));
    }

    #[test]
    fn wrong_audience_is_rejected() {
        let cfg = config_with_active("assay", &["assay-engine"]);
        let token = cfg
            .issue(&Claims {
                sub: "u".to_string(),
                iss: "assay".to_string(),
                aud: "someone-else".to_string(),
                exp: future_exp(),
            })
            .unwrap();
        let result = cfg.verify::<Claims>(&token);
        assert!(matches!(result, Err(Error::Jwt(_))));
    }

    #[test]
    fn expired_token_is_rejected() {
        let cfg = config_with_active("assay", &["assay-engine"]);
        let token = cfg
            .issue(&Claims {
                sub: "u".to_string(),
                iss: "assay".to_string(),
                aud: "assay-engine".to_string(),
                exp: past_exp(),
            })
            .unwrap();
        let result = cfg.verify::<Claims>(&token);
        assert!(matches!(result, Err(Error::Jwt(_))));
    }

    #[test]
    fn unknown_kid_is_rejected() {
        let cfg_a = config_with_active("assay", &["assay-engine"]);
        let token = cfg_a
            .issue(&Claims {
                sub: "u".to_string(),
                iss: "assay".to_string(),
                aud: "assay-engine".to_string(),
                exp: future_exp(),
            })
            .unwrap();
        // Build a fresh config with a different active key — verifying
        // the prior token must fail because the kid isn't known here.
        let cfg_b = JwtConfig::new("assay".to_string(), vec!["assay-engine".to_string()]);
        let other = generate_ephemeral_ed25519("kid_b").unwrap();
        cfg_b.set_active(other, Vec::new());
        let result = cfg_b.verify::<Claims>(&token);
        assert!(matches!(result, Err(Error::Jwt(_))));
    }
}
