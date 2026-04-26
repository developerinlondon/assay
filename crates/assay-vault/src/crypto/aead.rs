//! AES-256-GCM-SIV helpers — the single AEAD used everywhere in the
//! vault module. Centralising the wrapper makes "are we doing AEAD
//! correctly?" a one-file question for any future security review.

// `aes-gcm-siv` 0.11 still ships against `generic-array` 0.14, whose
// `GenericArray::from_slice` is marked deprecated in the 1.x line. There
// is no migration path until aes-gcm-siv ships a 1.x-compatible release;
// the calls below are correct for the version we depend on.
#![allow(deprecated)]

use aes_gcm_siv::aead::{Aead, KeyInit, Payload};
use aes_gcm_siv::{Aes256GcmSiv, Key, Nonce};
use rand::RngCore;

use crate::error::{Result, VaultError};

/// Nonce length required by AES-GCM-SIV (96 bits).
pub const NONCE_LEN: usize = 12;

/// 256-bit AEAD key length.
pub const KEY_LEN: usize = 32;

/// Encrypt `plaintext` with the given 256-bit key, 12-byte nonce, and
/// associated data. Returns the ciphertext (which includes the 16-byte
/// auth tag appended by the AEAD).
///
/// Returns `VaultError::Crypto` on AEAD failure — typically only happens
/// if the inputs themselves are malformed; the underlying primitive is
/// constant-time and infallible for valid sizes.
pub fn encrypt(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let cipher = Aes256GcmSiv::new(Key::<Aes256GcmSiv>::from_slice(key));
    cipher
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|e| VaultError::Crypto(format!("aead encrypt: {e}")))
}

/// Decrypt `ciphertext` with the given 256-bit key, 12-byte nonce, and
/// associated data. Returns `VaultError::Crypto` if the auth tag doesn't
/// validate — the most common failure mode is "the AAD changed since
/// the row was written" (e.g. KV row got moved to a different path).
pub fn decrypt(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    let cipher = Aes256GcmSiv::new(Key::<Aes256GcmSiv>::from_slice(key));
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|e| VaultError::Crypto(format!("aead decrypt: {e}")))
}

/// Random 12-byte nonce — for fresh-ciphertext write paths. AES-GCM-SIV
/// tolerates nonce reuse (that's the whole point of the SIV mode), so
/// we don't need the strict nonce-counter discipline plain GCM wants.
pub fn random_nonce() -> [u8; NONCE_LEN] {
    let mut n = [0u8; NONCE_LEN];
    rand::rng().fill_bytes(&mut n);
    n
}

/// Random 32-byte data-encryption key (DEK). Generated per KV record
/// and per transit-key-version; wrapped with the master KEK for at-rest
/// storage.
pub fn random_dek() -> [u8; KEY_LEN] {
    let mut k = [0u8; KEY_LEN];
    rand::rng().fill_bytes(&mut k);
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_with_aad() {
        let key = random_dek();
        let nonce = random_nonce();
        let aad = b"path=foo&version=1";
        let pt = b"hello world";
        let ct = encrypt(&key, &nonce, aad, pt).unwrap();
        let pt2 = decrypt(&key, &nonce, aad, &ct).unwrap();
        assert_eq!(pt2, pt);
    }

    #[test]
    fn aad_mismatch_fails() {
        let key = random_dek();
        let nonce = random_nonce();
        let pt = b"hello";
        let ct = encrypt(&key, &nonce, b"context-A", pt).unwrap();
        let res = decrypt(&key, &nonce, b"context-B", &ct);
        assert!(res.is_err(), "AEAD must reject AAD swap");
    }

    #[test]
    fn key_mismatch_fails() {
        let nonce = random_nonce();
        let aad = b"";
        let ct = encrypt(&random_dek(), &nonce, aad, b"hello").unwrap();
        let res = decrypt(&random_dek(), &nonce, aad, &ct);
        assert!(res.is_err());
    }

    #[test]
    fn ciphertext_tamper_fails() {
        let key = random_dek();
        let nonce = random_nonce();
        let mut ct = encrypt(&key, &nonce, b"", b"hello").unwrap();
        ct[0] ^= 1;
        assert!(decrypt(&key, &nonce, b"", &ct).is_err());
    }

    #[test]
    fn distinct_keys_per_call() {
        // Sanity — random_dek must not be a constant.
        let a = random_dek();
        let b = random_dek();
        assert_ne!(a, b);
    }
}
