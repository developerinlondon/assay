//! Master KEK handle.
//!
//! The KEK is a 256-bit symmetric key. Every DEK in KV (per-record),
//! every transit-key version, and every collection key envelope (Phase 3
//! server-side wrap fallback if used) is wrapped with this KEK.
//!
//! At rest the KEK lives in `vault.kek_metadata.sealed_blob`; how the
//! blob maps back to raw bytes depends on `sealing_method`:
//!
//! - `plaintext` (Phase 1 placeholder): blob *is* the 32 raw bytes.
//!   Engine boot logs a warning at INFO level so operators know to
//!   migrate to a real sealing method as Phase 2 lands.
//! - `shamir` / `kms-*` / `hsm` (Phase 2): real sealing — the unsealed
//!   bytes never touch disk; this handle holds them in memory only.
//!
//! The handle exposes envelope ops (wrap / unwrap a DEK) but never
//! exposes the raw bytes — every consumer goes through the wrap/unwrap
//! API. That confines the KEK material to this file.

use std::sync::Arc;

use crate::crypto::aead::{decrypt, encrypt, random_nonce, KEY_LEN, NONCE_LEN};
use crate::error::{Result, VaultError};

/// In-memory KEK material. Cheap to clone — the inner Arc shares the
/// raw bytes across consumers without re-allocating.
#[derive(Clone)]
#[non_exhaustive]
pub struct KekHandle {
    inner: Arc<KekInner>,
}

struct KekInner {
    kid: String,
    key: [u8; KEY_LEN],
}

/// Fully-resolved wrapped DEK — the bytes the storage layer puts on
/// disk. Layout: `nonce (12) || wrapped DEK ciphertext (32+16 = 48)`.
/// Total 60 bytes. Stored as a single BYTEA / BLOB column.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct WrappedDek(pub Vec<u8>);

impl WrappedDek {
    /// Convert from on-disk bytes (no validation — failures surface at
    /// unwrap time when the AEAD tag check fires).
    pub fn from_bytes(b: Vec<u8>) -> Self {
        Self(b)
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl KekHandle {
    /// Construct from raw key bytes + a stable `kid`. Caller is
    /// responsible for sourcing the bytes from a trusted path (engine
    /// boot's [`crate::crypto::kek_store`] helpers, or test code).
    pub fn from_bytes(kid: impl Into<String>, key: [u8; KEY_LEN]) -> Self {
        Self {
            inner: Arc::new(KekInner {
                kid: kid.into(),
                key,
            }),
        }
    }

    /// Generate a fresh ephemeral KEK with a content-addressed kid.
    /// Useful for tests + first-boot bootstrap before any persistence
    /// path runs.
    pub fn generate_ephemeral() -> Self {
        let key = crate::crypto::aead::random_dek();
        let kid = mint_kid(&key);
        Self::from_bytes(kid, key)
    }

    /// Stable identifier for this KEK — recorded in every `kek_kid`
    /// column so a future rotation can find which KEK wrapped each DEK.
    pub fn kid(&self) -> &str {
        &self.inner.kid
    }

    /// Wrap a DEK so it can be persisted at rest. The DEK itself is
    /// random per-record; this method just AEAD-encrypts those 32 bytes
    /// under the KEK with a fresh nonce. The kid binds the auth tag so
    /// a future "which KEK wrapped this?" lookup can sanity-check.
    pub fn wrap_dek(&self, dek: &[u8; KEY_LEN]) -> Result<WrappedDek> {
        let nonce = random_nonce();
        let aad = self.inner.kid.as_bytes();
        let ct = encrypt(&self.inner.key, &nonce, aad, dek)?;
        // Layout: nonce || ciphertext. Caller stores as a single blob.
        let mut buf = Vec::with_capacity(NONCE_LEN + ct.len());
        buf.extend_from_slice(&nonce);
        buf.extend_from_slice(&ct);
        Ok(WrappedDek(buf))
    }

    /// Unwrap a DEK previously produced by [`Self::wrap_dek`]. Fails if
    /// the kid is wrong (AAD mismatch) or the bytes are tampered (tag
    /// fails). The unwrapped DEK is returned by value — callers should
    /// keep it on the stack and let it drop at end of scope.
    pub fn unwrap_dek(&self, wrapped: &WrappedDek) -> Result<[u8; KEY_LEN]> {
        let bytes = wrapped.as_bytes();
        if bytes.len() < NONCE_LEN {
            return Err(VaultError::Crypto("wrapped DEK truncated".into()));
        }
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&bytes[..NONCE_LEN]);
        let ct = &bytes[NONCE_LEN..];
        let aad = self.inner.kid.as_bytes();
        let pt = decrypt(&self.inner.key, &nonce, aad, ct)?;
        if pt.len() != KEY_LEN {
            return Err(VaultError::Crypto(format!(
                "wrapped DEK plaintext is {} bytes; expected {}",
                pt.len(),
                KEY_LEN
            )));
        }
        let mut dek = [0u8; KEY_LEN];
        dek.copy_from_slice(&pt);
        Ok(dek)
    }
}

/// Content-address a KEK by hashing the public key material so different
/// KEKs get different kids without an external counter. Hex-encoded
/// SHA-256 truncated to 16 chars — 64 bits of collision resistance,
/// short enough to grep for in logs. The hash uses a domain-separation
/// label so pre-image attacks against unrelated SHA-256 outputs don't
/// transfer.
pub(crate) fn mint_kid(key: &[u8; KEY_LEN]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"assay-vault/kek-kid/v1");
    h.update(key);
    let digest = h.finalize();
    format!(
        "kek-{}",
        data_encoding::HEXLOWER_PERMISSIVE.encode(&digest[..8])
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::aead::random_dek;

    #[test]
    fn wrap_unwrap_round_trip() {
        let kek = KekHandle::generate_ephemeral();
        let dek = random_dek();
        let wrapped = kek.wrap_dek(&dek).unwrap();
        let dek2 = kek.unwrap_dek(&wrapped).unwrap();
        assert_eq!(dek, dek2);
    }

    #[test]
    fn unwrap_with_different_kek_fails() {
        let kek_a = KekHandle::generate_ephemeral();
        let kek_b = KekHandle::generate_ephemeral();
        let dek = random_dek();
        let wrapped = kek_a.wrap_dek(&dek).unwrap();
        assert!(
            kek_b.unwrap_dek(&wrapped).is_err(),
            "wrong KEK must fail to unwrap"
        );
    }

    #[test]
    fn unwrap_with_kid_collision_fails() {
        // Construct two KEKs with the same kid but different bytes — AAD
        // matches but key doesn't, so the tag check still fails.
        let key_a = random_dek();
        let key_b = random_dek();
        let kid = "kek-fixed";
        let kek_a = KekHandle::from_bytes(kid, key_a);
        let kek_b = KekHandle::from_bytes(kid, key_b);
        let wrapped = kek_a.wrap_dek(&random_dek()).unwrap();
        assert!(kek_b.unwrap_dek(&wrapped).is_err());
    }

    #[test]
    fn truncated_wrapped_dek_fails() {
        let kek = KekHandle::generate_ephemeral();
        let wrapped = WrappedDek(vec![0u8; 5]);
        assert!(kek.unwrap_dek(&wrapped).is_err());
    }

    #[test]
    fn kid_is_content_addressed() {
        let key = [42u8; KEY_LEN];
        let kek1 = KekHandle::from_bytes(mint_kid(&key), key);
        let kek2 = KekHandle::from_bytes(mint_kid(&key), key);
        assert_eq!(kek1.kid(), kek2.kid());

        let other = KekHandle::generate_ephemeral();
        assert_ne!(kek1.kid(), other.kid());
    }
}
