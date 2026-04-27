//! KV v2 — versioned, server-decryptable secrets storage. Vault-equivalent.
//!
//! Plan 17 §S1. Path-tree storage with version history, soft-delete,
//! hard-destroy, undelete, and arbitrary JSON metadata per path.
//!
//! ## Layering
//!
//! - [`KvStore`] is a trait of pure IO methods. The PG / SQLite impls
//!   in `store::postgres` / `store::sqlite` carry no crypto knowledge —
//!   they take and return raw bytes (`ciphertext`, `nonce`,
//!   `wrapped_dek`).
//! - [`KvService`] wraps a [`KvStore`] + [`KekHandle`] and is the
//!   surface every caller goes through. PUT mints a fresh DEK, encrypts
//!   the plaintext, wraps the DEK with the KEK, and atomically commits
//!   the metadata bump + row INSERT. GET unwraps and decrypts.
//!
//! That split lets a future Phase 2 KEK rotation re-encrypt every row
//! without touching the trait surface, and lets tests mock IO without
//! mocking crypto.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::crypto::aead::{decrypt, encrypt, random_dek, random_nonce};
use crate::crypto::kek::WrappedDek;
use crate::error::{Result, VaultError};

/// One stored version of a KV path. Returned by store-layer reads.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct KvRow {
    pub path: String,
    pub version: i64,
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub wrapped_dek: Vec<u8>,
    pub kek_kid: String,
    pub deleted_at: Option<f64>,
    pub destroyed: bool,
    pub created_at: f64,
}

/// Path-level metadata. One row in `vault.kv_meta` per path.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct KvMeta {
    pub path: String,
    pub latest_version: i64,
    pub custom_md: Value,
    pub created_at: f64,
    pub updated_at: f64,
}

/// Pure-IO trait — no crypto. Implementations live in `store::postgres`
/// / `store::sqlite`. The PUT path is one transaction: bump
/// `kv_meta.latest_version` and INSERT into `vault.kv` with that
/// version.
#[async_trait]
pub trait KvStore: Send + Sync + 'static {
    /// Atomically allocate the next version for `path`, INSERT the row,
    /// and merge `custom_md` into the path's metadata. Returns the
    /// allocated version.
    async fn put_row(
        &self,
        path: &str,
        ciphertext: &[u8],
        nonce: &[u8],
        wrapped_dek: &[u8],
        kek_kid: &str,
        custom_md: &Value,
    ) -> Result<i64>;

    /// Fetch a specific version. Returns None if the row never existed
    /// or was hard-destroyed (soft-deleted rows are still returned —
    /// the caller decides how to interpret `deleted_at`).
    async fn get_row(&self, path: &str, version: i64) -> Result<Option<KvRow>>;

    /// Fetch the latest non-destroyed row for `path`. The latest version
    /// being soft-deleted IS returned (with `deleted_at` set); call sites
    /// that want strict "live latest" semantics filter `deleted_at`
    /// themselves.
    async fn get_latest_row(&self, path: &str) -> Result<Option<KvRow>>;

    /// List path metadata under a prefix. Empty prefix lists every path.
    async fn list_meta(&self, prefix: &str) -> Result<Vec<KvMeta>>;

    /// Read one path's metadata.
    async fn read_meta(&self, path: &str) -> Result<Option<KvMeta>>;

    /// Soft-delete: set `deleted_at`. Idempotent. Returns whether a row
    /// was modified (false = already soft-deleted or hard-destroyed).
    async fn soft_delete(&self, path: &str, version: i64, deleted_at: f64) -> Result<bool>;

    /// Hard-destroy: zero out the ciphertext + wrapped_dek bytes and
    /// set `destroyed = TRUE`. The row stays so an audit trail can
    /// still answer "did v3 ever exist?" — only the secret material
    /// goes. Idempotent.
    async fn destroy(&self, path: &str, version: i64) -> Result<bool>;

    /// Reverse a soft-delete. Errors if the version was hard-destroyed
    /// — destroyed material can't be undone.
    async fn undelete(&self, path: &str, version: i64) -> Result<bool>;
}

/// Decrypted read result.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct KvRead {
    pub path: String,
    pub version: i64,
    pub plaintext: Vec<u8>,
    pub deleted_at: Option<f64>,
    pub created_at: f64,
}

/// High-level KV API. Wraps a store + the live sealing state. Cheap to
/// clone — store impls are `Arc<dyn KvStore>` underneath; `SealState`
/// is itself an `Arc`-shared inner. Every crypto op fetches the active
/// [`KekHandle`] via [`crate::crypto::seal_state::SealState::require_unsealed`],
/// which fails closed with [`VaultError::Sealed`] when the vault is
/// sealed.
#[derive(Clone)]
#[non_exhaustive]
pub struct KvService<S: KvStore> {
    store: S,
    seal_state: crate::crypto::seal_state::SealState,
}

impl<S: KvStore> KvService<S> {
    pub fn new(store: S, seal_state: crate::crypto::seal_state::SealState) -> Self {
        Self { store, seal_state }
    }

    /// Borrow the underlying store — useful for admin paths that need
    /// raw metadata reads without going through the crypto surface.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Active KEK kid (when unsealed). Returns `None` while sealed.
    pub fn kek_kid(&self) -> Option<String> {
        self.seal_state.require_unsealed().ok().map(|h| h.kid().to_string())
    }

    /// Encrypt and store a new version of `path`. Returns the allocated
    /// version. `custom_md` is merged into the path-level metadata; pass
    /// `Value::Null` or `serde_json::json!({})` to leave it untouched.
    pub async fn put(
        &self,
        path: &str,
        plaintext: &[u8],
        custom_md: Value,
    ) -> Result<i64> {
        validate_path(path)?;
        let kek = self.seal_state.require_unsealed()?;
        let dek = random_dek();
        let nonce = random_nonce();
        let aad = path_aad(path);
        let ciphertext = encrypt(&dek, &nonce, &aad, plaintext)?;
        let wrapped = kek.wrap_dek(&dek)?;
        let version = self
            .store
            .put_row(
                path,
                &ciphertext,
                &nonce,
                wrapped.as_bytes(),
                kek.kid(),
                &custom_md,
            )
            .await?;
        Ok(version)
    }

    /// Read a specific version (or the latest, when `version` is None).
    /// Returns `VaultError::NotFound` if the path / version doesn't
    /// exist; soft-deleted versions are returned with `deleted_at` set
    /// so the caller can choose to surface "this secret was deleted".
    pub async fn get(&self, path: &str, version: Option<i64>) -> Result<KvRead> {
        validate_path(path)?;
        let kek = self.seal_state.require_unsealed()?;
        let row = match version {
            Some(v) => self.store.get_row(path, v).await?,
            None => self.store.get_latest_row(path).await?,
        };
        let row = row.ok_or(VaultError::NotFound)?;
        if row.destroyed {
            return Err(VaultError::NotFound);
        }
        if row.kek_kid != kek.kid() {
            // Row was wrapped under a different KEK kid than the
            // currently-active one. KEK rotation (Phase 2 follow-up)
            // re-wraps every DEK; until that ships, refuse rather than
            // silently fail.
            return Err(VaultError::Crypto(format!(
                "row encrypted with KEK {kid} but service active KEK is {active}",
                kid = row.kek_kid,
                active = kek.kid()
            )));
        }
        let dek = kek.unwrap_dek(&WrappedDek::from_bytes(row.wrapped_dek.clone()))?;
        let aad = path_aad(&row.path);
        let mut nonce = [0u8; 12];
        if row.nonce.len() != nonce.len() {
            return Err(VaultError::Crypto(format!(
                "row nonce is {} bytes; expected 12",
                row.nonce.len()
            )));
        }
        nonce.copy_from_slice(&row.nonce);
        let plaintext = decrypt(&dek, &nonce, &aad, &row.ciphertext)?;
        Ok(KvRead {
            path: row.path,
            version: row.version,
            plaintext,
            deleted_at: row.deleted_at,
            created_at: row.created_at,
        })
    }

    pub async fn list(&self, prefix: &str) -> Result<Vec<KvMeta>> {
        self.store.list_meta(prefix).await
    }

    pub async fn read_meta(&self, path: &str) -> Result<KvMeta> {
        validate_path(path)?;
        self.store
            .read_meta(path)
            .await?
            .ok_or(VaultError::NotFound)
    }

    pub async fn soft_delete(&self, path: &str, version: i64) -> Result<()> {
        validate_path(path)?;
        let now = unix_now();
        let modified = self.store.soft_delete(path, version, now).await?;
        if !modified {
            return Err(VaultError::NotFound);
        }
        Ok(())
    }

    pub async fn destroy(&self, path: &str, version: i64) -> Result<()> {
        validate_path(path)?;
        let modified = self.store.destroy(path, version).await?;
        if !modified {
            return Err(VaultError::NotFound);
        }
        Ok(())
    }

    pub async fn undelete(&self, path: &str, version: i64) -> Result<()> {
        validate_path(path)?;
        let modified = self.store.undelete(path, version).await?;
        if !modified {
            return Err(VaultError::NotFound);
        }
        Ok(())
    }
}

/// AAD bound into the AEAD for every KV row. Path becomes part of the
/// auth-tag input so a row physically moved to a different path fails
/// to authenticate. Phase 2 may extend this with the KEK kid; we keep
/// it minimal for now so KV operations don't depend on KEK rotation
/// state.
fn path_aad(path: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(b"vault.kv:".len() + path.len());
    buf.extend_from_slice(b"vault.kv:");
    buf.extend_from_slice(path.as_bytes());
    buf
}

/// Validate a KV path. Phase 1 keeps the rules minimal: non-empty, no
/// NUL bytes, length-bounded so a hostile caller can't OOM the meta
/// table. Stricter shape (e.g. forbidding leading slash) lands when
/// the HTTP handlers do their own path-shape work in the route layer.
fn validate_path(path: &str) -> Result<()> {
    if path.is_empty() {
        return Err(VaultError::Invalid("kv path is empty".into()));
    }
    if path.len() > 1024 {
        return Err(VaultError::Invalid("kv path > 1024 bytes".into()));
    }
    if path.as_bytes().contains(&0) {
        return Err(VaultError::Invalid("kv path contains NUL".into()));
    }
    Ok(())
}

fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_aad_includes_path() {
        let a = path_aad("foo");
        let b = path_aad("bar");
        assert_ne!(a, b);
        assert!(a.starts_with(b"vault.kv:"));
    }

    #[test]
    fn validate_path_rejects_obvious_garbage() {
        assert!(validate_path("").is_err());
        assert!(validate_path("foo\0bar").is_err());
        let huge = "a".repeat(2000);
        assert!(validate_path(&huge).is_err());
        assert!(validate_path("api/stripe/webhook").is_ok());
    }
}
