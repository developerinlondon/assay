//! Biscuit capability tokens — public-key signed bearers with
//! Datalog policy, offline verification, and caller-side attenuation.
//!
//! Plan 11 reference: `biscuit-auth` 6 — public-key signed, Datalog
//! policy, offline verifiable, attenuable. Phase 5 + correction (per
//! coordinator): biscuit is foundational, NOT feature-gated, and ships
//! in every build that pulls assay-auth in.
//!
//! Lifecycle:
//!
//! 1. Boot loads the active root keypair from `auth.biscuit_root_keys`
//!    (the row with `rotated_at IS NULL`); if no row exists, generates
//!    a fresh Ed25519 keypair and INSERTs it. Mirrors the JWKS bootstrap
//!    pattern in [`crate::jwt`].
//! 2. [`BiscuitConfig::issue`] signs a fresh authority block via the
//!    builder closure passed by the caller, returning a base64-encoded
//!    URL-safe token ready for `Authorization: Bearer …`.
//! 3. [`BiscuitConfig::verify`] base64-decodes, validates the signature
//!    against the cached root public key, and runs a caller-supplied
//!    [`Authorizer`] (policies + checks).
//! 4. [`BiscuitConfig::attenuate`] is a free function — caller-side, no
//!    root key required. Appends a more-restrictive block and
//!    re-encodes. The result is a valid biscuit that any verifier with
//!    the same root public key can validate without re-contacting
//!    assay-engine.

use std::sync::Arc;

use biscuit_auth::builder::AuthorizerBuilder;
use biscuit_auth::{Biscuit, BiscuitBuilder, BlockBuilder, KeyPair, PublicKey};
use parking_lot::RwLock;

use crate::error::{Error, Result};

/// Active root key + history. The active row signs new tokens; history
/// rows still verify previously-issued tokens until they expire on
/// their own.
struct Inner {
    active: ActiveRootKey,
    history: Vec<HistoryRootKey>,
}

/// Active root key — signs new biscuits, also verifies them.
pub struct ActiveRootKey {
    pub kid: String,
    pub keypair: KeyPair,
}

/// Verify-only entry — older root keys retained so previously-issued
/// biscuits validate after a rotation. We don't track a `kid` per
/// biscuit yet (biscuit-auth 6 carries an optional `root_key_id`); for
/// now we attempt the active key first then fall through history.
pub struct HistoryRootKey {
    pub kid: String,
    pub public_key: PublicKey,
}

/// Cheap-to-clone biscuit configuration. Wraps the active keypair +
/// history behind an [`RwLock`] so a future `rotate` lands without
/// breaking inflight callers.
#[derive(Clone)]
pub struct BiscuitConfig {
    inner: Arc<RwLock<Inner>>,
}

impl BiscuitConfig {
    /// Construct from an explicit active root keypair. Useful for tests
    /// and for engine boot's "load row, build config" path.
    pub fn from_active(active: ActiveRootKey, history: Vec<HistoryRootKey>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner { active, history })),
        }
    }

    /// Generate a fresh ephemeral Ed25519 root keypair without touching
    /// any DB. The default for [`crate::ctx::AuthCtx::new`] callers
    /// that don't have a persistent root key yet — engine boot replaces
    /// this with the loaded-or-generated row via
    /// [`crate::ctx::AuthCtx::with_biscuit`].
    pub fn generate_ephemeral() -> Self {
        let keypair = KeyPair::new();
        let kid = mint_kid(&keypair.public());
        Self::from_active(ActiveRootKey { kid, keypair }, Vec::new())
    }

    /// Construct from an existing root keypair PEM (the format
    /// [`KeyPair::to_private_key_pem`] emits). Used by engine boot
    /// when the `auth.biscuit_root_keys` row carries a stored private
    /// key.
    pub fn from_pem(pem: &str) -> Result<Self> {
        let keypair = KeyPair::from_private_key_pem(pem)
            .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit root key from pem: {e}")))?;
        let kid = mint_kid(&keypair.public());
        Ok(Self::from_active(
            ActiveRootKey { kid, keypair },
            Vec::new(),
        ))
    }

    /// Borrow the active root key id (kid). Cheap; clones one short
    /// string under the read lock.
    pub fn active_kid(&self) -> String {
        self.inner.read().active.kid.clone()
    }

    /// Render the active root public key as a PEM string for
    /// distribution to standalone verifiers (mobile clients, edge
    /// services). Stable as long as the active row in
    /// `auth.biscuit_root_keys` doesn't rotate.
    pub fn public_pem(&self) -> Result<String> {
        self.inner
            .read()
            .active
            .keypair
            .public()
            .to_pem()
            .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit public pem: {e}")))
    }

    /// Borrow the active root public key. Useful for test
    /// reconstruction and for the public_pem helper.
    pub fn active_public_key(&self) -> PublicKey {
        self.inner.read().active.keypair.public()
    }

    /// Issue a fresh biscuit via the supplied builder closure. The
    /// closure receives an empty [`BiscuitBuilder`] and returns the
    /// completed builder; we sign + base64-URL-encode it for the wire.
    ///
    /// Example:
    /// ```ignore
    /// let token = cfg.issue(|b| b.fact("user(\"alice\")"))?;
    /// ```
    pub fn issue<F>(&self, build: F) -> Result<String>
    where
        F: FnOnce(
            BiscuitBuilder,
        ) -> std::result::Result<BiscuitBuilder, biscuit_auth::error::Token>,
    {
        let builder = build(Biscuit::builder())
            .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit build: {e}")))?;
        let guard = self.inner.read();
        let token = builder
            .build(&guard.active.keypair)
            .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit sign: {e}")))?;
        token
            .to_base64()
            .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit base64: {e}")))
    }

    /// Verify a biscuit and run the supplied authorizer against it. The
    /// closure receives a fresh [`AuthorizerBuilder`]; add policies +
    /// checks via its builder methods, returning the completed builder.
    /// We then build the authorizer against the parsed token and call
    /// `authorize`.
    ///
    /// `Ok(())` means the token was syntactically valid, signed by a
    /// known root key, and matched at least one allow policy without
    /// triggering any deny / failed check.
    pub fn verify<F>(&self, token: &str, build: F) -> Result<()>
    where
        F: FnOnce(
            AuthorizerBuilder,
        ) -> std::result::Result<AuthorizerBuilder, biscuit_auth::error::Token>,
    {
        let guard = self.inner.read();
        // Try the active key first; on signature mismatch, fall through
        // to history (each history row is a previously-active root).
        let parsed = match Biscuit::from_base64(token, guard.active.keypair.public()) {
            Ok(t) => t,
            Err(active_err) => {
                let mut last = active_err;
                let mut found = None;
                for hist in &guard.history {
                    match Biscuit::from_base64(token, hist.public_key) {
                        Ok(t) => {
                            found = Some(t);
                            break;
                        }
                        Err(e) => last = e,
                    }
                }
                match found {
                    Some(t) => t,
                    None => {
                        return Err(Error::Backend(anyhow::anyhow!(
                            "biscuit signature verify: {last}"
                        )));
                    }
                }
            }
        };
        let authorizer_builder = build(AuthorizerBuilder::new())
            .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit authorizer build: {e}")))?;
        let mut authorizer = authorizer_builder
            .build(&parsed)
            .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit authorizer attach: {e}")))?;
        authorizer
            .authorize()
            .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit authorize: {e}")))?;
        Ok(())
    }
}

/// Caller-side attenuation. Anyone with the bearer token (and no
/// access to the root keypair) can append a new block of restrictions.
/// The result is a valid biscuit that the original verifier accepts.
///
/// `root_public` is needed to parse the source token; for assay's
/// in-process callers this is [`BiscuitConfig::active_public_key`].
/// Standalone clients pass the PEM-loaded [`PublicKey`] from
/// distribution.
pub fn attenuate<F>(token: &str, root_public: PublicKey, build: F) -> Result<String>
where
    F: FnOnce(BlockBuilder) -> std::result::Result<BlockBuilder, biscuit_auth::error::Token>,
{
    let parsed = Biscuit::from_base64(token, root_public)
        .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit parse for attenuate: {e}")))?;
    let block = build(BlockBuilder::new())
        .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit block build: {e}")))?;
    let attenuated = parsed
        .append(block)
        .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit append block: {e}")))?;
    attenuated
        .to_base64()
        .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit base64 (attenuated): {e}")))
}

/// Postgres bootstrap helper. Loads the active root key from
/// `auth.biscuit_root_keys`; if no row exists, generates a fresh
/// Ed25519 keypair, persists it, and returns a config that uses it.
///
/// Called from engine boot. Mirrors the JWKS bootstrap pattern in
/// [`crate::jwt::JwtConfig::load_from_postgres`] / `rotate_postgres`.
#[cfg(feature = "backend-postgres")]
pub async fn load_or_init_postgres(pool: &sqlx::PgPool) -> Result<BiscuitConfig> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT kid, private_pem
         FROM auth.biscuit_root_keys
         WHERE rotated_at IS NULL
         ORDER BY created_at DESC
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| Error::Backend(anyhow::anyhow!("load auth.biscuit_root_keys (pg): {e}")))?;

    if let Some(row) = row {
        let kid: String = row.get("kid");
        let pem_bytes: Vec<u8> = row.get("private_pem");
        let pem = std::str::from_utf8(&pem_bytes)
            .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit private_pem utf8: {e}")))?;
        let keypair = KeyPair::from_private_key_pem(pem)
            .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit from_private_key_pem: {e}")))?;
        Ok(BiscuitConfig::from_active(
            ActiveRootKey { kid, keypair },
            Vec::new(),
        ))
    } else {
        let keypair = KeyPair::new();
        let kid = mint_kid(&keypair.public());
        let private_pem = keypair
            .to_private_key_pem()
            .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit to_private_key_pem: {e}")))?
            .to_string();
        let public_pem = keypair
            .public()
            .to_pem()
            .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit public to_pem: {e}")))?;
        let now = now_secs();
        sqlx::query(
            "INSERT INTO auth.biscuit_root_keys
                 (kid, private_pem, public_pem, created_at, rotated_at)
             VALUES ($1, $2, $3, $4, NULL)",
        )
        .bind(&kid)
        .bind(private_pem.as_bytes())
        .bind(&public_pem)
        .bind(now)
        .execute(pool)
        .await
        .map_err(|e| Error::Backend(anyhow::anyhow!("insert auth.biscuit_root_keys (pg): {e}")))?;
        Ok(BiscuitConfig::from_active(
            ActiveRootKey { kid, keypair },
            Vec::new(),
        ))
    }
}

/// SQLite mirror of [`load_or_init_postgres`].
#[cfg(feature = "backend-sqlite")]
pub async fn load_or_init_sqlite(pool: &sqlx::SqlitePool) -> Result<BiscuitConfig> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT kid, private_pem
         FROM auth.biscuit_root_keys
         WHERE rotated_at IS NULL
         ORDER BY created_at DESC
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| Error::Backend(anyhow::anyhow!("load auth.biscuit_root_keys (sqlite): {e}")))?;

    if let Some(row) = row {
        let kid: String = row.get("kid");
        let pem_bytes: Vec<u8> = row.get("private_pem");
        let pem = std::str::from_utf8(&pem_bytes)
            .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit private_pem utf8: {e}")))?;
        let keypair = KeyPair::from_private_key_pem(pem)
            .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit from_private_key_pem: {e}")))?;
        Ok(BiscuitConfig::from_active(
            ActiveRootKey { kid, keypair },
            Vec::new(),
        ))
    } else {
        let keypair = KeyPair::new();
        let kid = mint_kid(&keypair.public());
        let private_pem = keypair
            .to_private_key_pem()
            .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit to_private_key_pem: {e}")))?
            .to_string();
        let public_pem = keypair
            .public()
            .to_pem()
            .map_err(|e| Error::Backend(anyhow::anyhow!("biscuit public to_pem: {e}")))?;
        let now = now_secs();
        sqlx::query(
            "INSERT INTO auth.biscuit_root_keys
                 (kid, private_pem, public_pem, created_at, rotated_at)
             VALUES (?, ?, ?, ?, NULL)",
        )
        .bind(&kid)
        .bind(private_pem.as_bytes())
        .bind(&public_pem)
        .bind(now)
        .execute(pool)
        .await
        .map_err(|e| {
            Error::Backend(anyhow::anyhow!(
                "insert auth.biscuit_root_keys (sqlite): {e}"
            ))
        })?;
        Ok(BiscuitConfig::from_active(
            ActiveRootKey { kid, keypair },
            Vec::new(),
        ))
    }
}

/// Mint a deterministic-ish kid from the public key bytes — short
/// base64 prefix so two distinct keypairs collide with negligible
/// probability and operators can eyeball-correlate rows. Stable: same
/// input bytes yield the same kid.
fn mint_kid(public_key: &PublicKey) -> String {
    let bytes = public_key.to_bytes();
    let prefix = if bytes.len() >= 16 {
        &bytes[..16]
    } else {
        &bytes
    };
    format!("kid_{}", data_encoding::BASE64URL_NOPAD.encode(prefix))
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> BiscuitConfig {
        BiscuitConfig::generate_ephemeral()
    }

    #[test]
    fn issue_and_verify_round_trip() {
        let cfg = cfg();
        let token = cfg
            .issue(|b| {
                b.fact("user(\"alice\")")
                    .and_then(|b| b.fact("role(\"admin\")"))
            })
            .expect("issue");
        cfg.verify(&token, |a| a.policy("allow if user(\"alice\")"))
            .expect("verify");
    }

    #[test]
    fn verify_rejects_tampered_token() {
        let cfg = cfg();
        let token = cfg.issue(|b| b.fact("user(\"alice\")")).expect("issue");
        // Flip a single byte mid-token; the signature verify should fail.
        let mut bytes = token.into_bytes();
        let half = bytes.len() / 2;
        bytes[half] ^= 0x01;
        let tampered = String::from_utf8_lossy(&bytes).to_string();
        let result = cfg.verify(&tampered, |a| a.policy("allow if user(\"alice\")"));
        assert!(matches!(result, Err(Error::Backend(_))));
    }

    #[test]
    fn verify_with_unknown_root_key_fails() {
        // Issue with cfg_a, verify with cfg_b — different root keypair.
        let cfg_a = cfg();
        let cfg_b = cfg();
        let token = cfg_a.issue(|b| b.fact("user(\"alice\")")).expect("issue");
        let result = cfg_b.verify(&token, |a| a.policy("allow if user(\"alice\")"));
        assert!(matches!(result, Err(Error::Backend(_))));
    }

    #[test]
    fn attenuate_produces_valid_child_token() {
        let cfg = cfg();
        let token = cfg.issue(|b| b.fact("user(\"alice\")")).expect("issue");
        let pubkey = cfg.active_public_key();
        // Attenuate with an extra fact + a check: reading must be
        // explicitly allowed.
        let attenuated = attenuate(&token, pubkey, |b| b.check("check if operation(\"read\")"))
            .expect("attenuate");
        // With operation("read") → allowed.
        cfg.verify(&attenuated, |a| {
            a.fact("operation(\"read\")")
                .and_then(|a| a.policy("allow if user(\"alice\")"))
        })
        .expect("read should pass");
        // Without operation("read") → check fails (no fact => check
        // unsatisfied), authorize errors.
        let result = cfg.verify(&attenuated, |a| a.policy("allow if user(\"alice\")"));
        assert!(matches!(result, Err(Error::Backend(_))));
    }

    #[test]
    fn time_based_check_rejects_after_expiry() {
        let cfg = cfg();
        // Issue an unrestricted token; the attenuation pins a time check.
        let token = cfg.issue(|b| b.fact("user(\"alice\")")).expect("issue");
        let pubkey = cfg.active_public_key();
        let attenuated = attenuate(&token, pubkey, |b| {
            // Note: time/2026 in past is rejected; pin to year 2000 so
            // any "now" the test runs in is past expiry.
            b.check("check if time($now), $now < 2000-01-01T00:00:00Z")
        })
        .expect("attenuate");
        let result = cfg.verify(&attenuated, |a| a.time().policy("allow if user(\"alice\")"));
        assert!(matches!(result, Err(Error::Backend(_))));
    }

    #[test]
    fn from_pem_round_trips() {
        let cfg = cfg();
        let pem = cfg
            .inner
            .read()
            .active
            .keypair
            .to_private_key_pem()
            .expect("to_private_key_pem")
            .to_string();
        let restored = BiscuitConfig::from_pem(&pem).expect("from_pem");
        // The restored keypair should produce the same public key.
        assert_eq!(
            restored.active_public_key().to_bytes(),
            cfg.active_public_key().to_bytes(),
        );
    }

    #[test]
    fn public_pem_is_non_empty_and_pem_shaped() {
        let cfg = cfg();
        let pem = cfg.public_pem().expect("public_pem");
        assert!(pem.contains("PUBLIC KEY"), "got: {pem}");
    }

    #[test]
    fn active_kid_is_stable_across_clones() {
        let cfg = cfg();
        let kid = cfg.active_kid();
        let dup = cfg.clone();
        assert_eq!(kid, dup.active_kid());
    }
}
