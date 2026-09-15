//! Sealing the master KEK with operator-supplied unseal material.
//!
//! Without it the KEK sits in `vault.kek_metadata.sealed_blob` as raw
//! bytes, so a database dump is a plaintext copy of every vault secret.
//! With it the dump carries only ciphertext, and the key to read it
//! lives wherever the deployment keeps its secrets.
//!
//! This module owns the crypto only: deriving a 32-byte key from a
//! string, and sealing / unsealing the KEK under it. *Where* the string
//! comes from is [`crate::crypto::seal_source`], and what boot does when
//! there is none is [`crate::crypto::seal_policy`]. The environment
//! variable this module is named for is now one source among several,
//! and stays the default.
//!
//! ## What zeroizing here does and does not buy
//!
//! [`SealKey`] scrubs its bytes on drop, as do the KEK handle and the
//! plaintext intermediates in [`crate::crypto::kek_store`]. That narrows
//! the window in which a freed allocation still holds key material. It
//! does not close it: `aes-gcm-siv` keeps its own expanded round keys,
//! a `Vec` that reallocated leaves its old buffer behind untouched, and
//! nothing here stops the whole process being paged to swap or written
//! to a core dump. Treat it as one layer, not as a guarantee.

use crate::crypto::aead::{KEY_LEN, NONCE_LEN, decrypt, encrypt, random_nonce};
use crate::error::{Result, VaultError};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

/// Value written to `vault.kek_metadata.sealing_method`.
///
/// It names the *blob layout*, not the source the key came from: a store
/// sealed from a file or a passphrase records this too, because the
/// bytes on disk are identical and the same value must keep opening
/// stores sealed before other sources existed. Changing it would be a
/// data migration for no gain.
pub const METHOD_ENV: &str = "env-aes-gcm";

/// The environment variable read when `[vault.sealing]` names no other
/// source. Still the default, and still the only source that is allowed
/// to be silently absent.
pub const ENV_VAR: &str = "ASSAY_VAULT_SEAL_KEY";

/// Shortest accepted value. The seal key is whatever string the
/// deployment supplies; the floor is what stops a guessable one.
pub const MIN_CHARS: usize = 32;

/// Domain separation, so the derived key is specific to this use.
const DERIVE_LABEL: &[u8] = b"assay-vault/env-seal/v1";

/// Leading byte of `sealed_blob`, so a future layout can be told apart
/// from this one without a schema change.
const BLOB_VERSION: u8 = 1;

/// A seal key held in memory for the life of the process.
///
/// `origin` is the operator-facing name of wherever the material came
/// from — an environment variable, a file path, a config field. It is
/// carried so that a store which fails to open names the thing to go
/// and check, rather than always naming the variable even when the
/// deployment reads its key from a file.
#[derive(Clone)]
pub struct SealKey {
    key: [u8; KEY_LEN],
    /// Not secret, and not scrubbed — it is a label like
    /// `ASSAY_VAULT_SEAL_KEY` or a path already present in the config.
    origin: String,
}

/// Written out rather than derived so it is obvious that `origin` is
/// deliberately left alone: only `key` is material.
impl Drop for SealKey {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

impl std::fmt::Debug for SealKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SealKey(redacted, from {})", self.origin)
    }
}

impl SealKey {
    /// Derive the sealing key from a string supplied through
    /// [`ENV_VAR`]. See [`Self::derive_from`] for the derivation; this
    /// is the shorthand for the default source.
    pub fn derive(raw: &str) -> Result<Self> {
        Self::derive_from(raw, ENV_VAR)
    }

    /// Derive the sealing key from whatever string `origin` supplied, by
    /// SHA-256 over a fixed label and the value.
    ///
    /// Deriving rather than decoding means the encoding does not matter:
    /// base64, hex or a passphrase all work, and a value that happens to
    /// be valid base64 is not silently treated as one. It also means the
    /// same string produces the same key whichever source delivered it,
    /// so an operator can move a secret from the environment into a file
    /// without re-sealing the store. Surrounding whitespace is trimmed,
    /// because a secret delivered as a file usually arrives with a
    /// trailing newline.
    pub fn derive_from(raw: &str, origin: impl Into<String>) -> Result<Self> {
        let origin = origin.into();
        let trimmed = raw.trim();
        let length = trimmed.chars().count();
        if length < MIN_CHARS {
            return Err(VaultError::Invalid(format!(
                "{origin} must be at least {MIN_CHARS} characters, got {length}"
            )));
        }
        let mut hasher = <Sha256 as Digest>::new();
        hasher.update(DERIVE_LABEL);
        hasher.update(trimmed.as_bytes());
        Ok(Self {
            key: hasher.finalize().into(),
            origin,
        })
    }

    /// Wrap 32 bytes that are already a uniformly-random key — the
    /// output of a KDF, which needs no further hashing. The character
    /// floor [`Self::derive_from`] applies does not apply here: the
    /// caller is responsible for the strength of what it fed the KDF.
    pub fn from_key_bytes(key: [u8; KEY_LEN], origin: impl Into<String>) -> Self {
        Self {
            key,
            origin: origin.into(),
        }
    }

    /// Where this key's material came from, for operator-facing errors.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Read the seal key from the environment. `Ok(None)` means the
    /// variable is absent or empty; a malformed value is an error rather
    /// than a silent `None`. What an absent key means for boot is
    /// [`crate::crypto::seal_policy::SealPolicy`]'s decision, not this
    /// function's.
    pub fn from_env() -> Result<Option<Self>> {
        match std::env::var(ENV_VAR) {
            Ok(raw) if !raw.trim().is_empty() => Self::derive(&raw).map(Some),
            _ => Ok(None),
        }
    }

    /// Seal `kek` for storage. The kid is the AAD, so a blob cannot be
    /// moved to another row without the unseal failing.
    pub fn seal(&self, kid: &str, kek: &[u8; KEY_LEN]) -> Result<Vec<u8>> {
        let nonce = random_nonce();
        let ciphertext = encrypt(&self.key, &nonce, kid.as_bytes(), kek)?;
        let mut blob = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
        blob.push(BLOB_VERSION);
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);
        Ok(blob)
    }

    pub fn unseal(&self, kid: &str, blob: &[u8]) -> Result<[u8; KEY_LEN]> {
        let (&version, rest) = blob
            .split_first()
            .ok_or_else(|| VaultError::Crypto("sealed KEK blob is empty".into()))?;
        if version != BLOB_VERSION {
            return Err(VaultError::Crypto(format!(
                "sealed KEK blob version {version} is not supported"
            )));
        }
        if rest.len() <= NONCE_LEN {
            return Err(VaultError::Crypto("sealed KEK blob is truncated".into()));
        }
        let (nonce, ciphertext) = rest.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce.try_into().expect("split at NONCE_LEN");
        let origin = &self.origin;
        let plain = Zeroizing::new(
            decrypt(&self.key, &nonce, kid.as_bytes(), ciphertext).map_err(|_| {
                VaultError::Crypto(format!(
                    "the KEK for kid={kid} does not decrypt under {origin}; \
                     check the seal key matches the one that sealed this store"
                ))
            })?,
        );
        plain.as_slice().try_into().map_err(|_| {
            VaultError::Crypto(format!("sealed KEK unwrapped to {} bytes", plain.len()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plausible value: the chart generates base64 of 32 random bytes.
    const RAW: &str = "YzBmZTFhMmIzYzRkNWU2ZjcwODE5MmEzYjRjNWQ2ZTc=";
    const OTHER: &str = "ZzBmZTFhMmIzYzRkNWU2ZjcwODE5MmEzYjRjNWQ2ZTc=";

    fn key() -> SealKey {
        SealKey::derive(RAW).unwrap()
    }

    #[test]
    fn a_sealed_kek_comes_back_unchanged() {
        let kek = [42u8; KEY_LEN];
        let blob = key().seal("kek-abc", &kek).unwrap();
        assert_ne!(
            &blob[1 + NONCE_LEN..],
            &kek[..],
            "the blob must be ciphertext"
        );
        assert_eq!(key().unseal("kek-abc", &blob).unwrap(), kek);
    }

    #[test]
    fn the_same_string_always_derives_the_same_key() {
        let blob = SealKey::derive(RAW)
            .unwrap()
            .seal("kek-abc", &[7u8; KEY_LEN])
            .unwrap();
        let reopened = SealKey::derive(RAW)
            .unwrap()
            .unseal("kek-abc", &blob)
            .unwrap();
        assert_eq!(reopened, [7u8; KEY_LEN]);
    }

    /// The two constants differ only in their first character.
    #[test]
    fn one_character_of_difference_opens_nothing() {
        assert_eq!(RAW.len(), OTHER.len());
        assert_eq!(
            RAW.chars()
                .zip(OTHER.chars())
                .filter(|(a, b)| a != b)
                .count(),
            1,
            "the fixtures must differ in exactly one character"
        );
        let blob = key().seal("kek-abc", &[42u8; KEY_LEN]).unwrap();
        let err = SealKey::derive(OTHER)
            .unwrap()
            .unseal("kek-abc", &blob)
            .unwrap_err();
        assert!(err.to_string().contains(ENV_VAR), "{err}");
    }

    #[test]
    fn surrounding_whitespace_does_not_change_the_key() {
        let blob = key().seal("kek-abc", &[3u8; KEY_LEN]).unwrap();
        let padded = SealKey::derive(&format!("  {RAW}\n")).unwrap();
        assert_eq!(padded.unseal("kek-abc", &blob).unwrap(), [3u8; KEY_LEN]);
    }

    #[test]
    fn the_encoding_is_irrelevant_so_a_passphrase_works() {
        let phrase = "correct horse battery staple correct horse";
        let blob = SealKey::derive(phrase)
            .unwrap()
            .seal("kek-abc", &[5u8; KEY_LEN])
            .unwrap();
        let out = SealKey::derive(phrase)
            .unwrap()
            .unseal("kek-abc", &blob)
            .unwrap();
        assert_eq!(out, [5u8; KEY_LEN]);
    }

    #[test]
    fn a_blob_moved_to_another_kid_does_not_open() {
        let blob = key().seal("kek-abc", &[42u8; KEY_LEN]).unwrap();
        assert!(key().unseal("kek-other", &blob).is_err());
    }

    #[test]
    fn a_short_seal_key_is_refused_at_the_boundary() {
        let short = "a".repeat(MIN_CHARS - 1);
        let err = SealKey::derive(&short).unwrap_err();
        assert!(err.to_string().contains("at least 32 characters"), "{err}");
        assert!(
            SealKey::derive(&"a".repeat(MIN_CHARS)).is_ok(),
            "exactly the minimum must be accepted"
        );
    }

    #[test]
    fn a_truncated_blob_is_refused_rather_than_panicking() {
        let blob = key().seal("kek-abc", &[42u8; KEY_LEN]).unwrap();
        assert!(key().unseal("kek-abc", &blob[..NONCE_LEN]).is_err());
        assert!(key().unseal("kek-abc", &[]).is_err());
    }

    /// The same string is the same key however it reached the process,
    /// so an operator can move a secret from the environment into a file
    /// without re-sealing the store.
    #[test]
    fn the_source_a_value_arrived_from_does_not_change_the_key() {
        let blob = key().seal("kek-abc", &[9u8; KEY_LEN]).unwrap();
        let from_file = SealKey::derive_from(RAW, "the seal key file /run/secrets/seal").unwrap();
        assert_eq!(from_file.unseal("kek-abc", &blob).unwrap(), [9u8; KEY_LEN]);
    }

    /// A store that will not open names the place to go and look, which
    /// is not always the variable.
    #[test]
    fn a_failure_to_open_names_the_source_the_key_came_from() {
        let blob = key().seal("kek-abc", &[42u8; KEY_LEN]).unwrap();
        let elsewhere = SealKey::derive_from(OTHER, "the seal key file /run/secrets/seal").unwrap();
        let err = elsewhere.unseal("kek-abc", &blob).unwrap_err();
        assert!(err.to_string().contains("/run/secrets/seal"), "{err}");
        assert!(!err.to_string().contains(ENV_VAR), "{err}");
    }

    /// `Debug` is what a containing struct's derive would print, so it
    /// must not be the key.
    #[test]
    fn debug_shows_the_source_but_never_the_key() {
        let rendered = format!("{:?}", key());
        assert!(rendered.contains("redacted"), "{rendered}");
        assert!(rendered.contains(ENV_VAR), "{rendered}");
        assert!(!rendered.contains(RAW), "{rendered}");
    }

    #[test]
    fn nonces_do_not_repeat_across_seals() {
        let k = key();
        let a = k.seal("kek-abc", &[42u8; KEY_LEN]).unwrap();
        let b = k.seal("kek-abc", &[42u8; KEY_LEN]).unwrap();
        assert_ne!(a, b, "each seal must use a fresh nonce");
    }
}
