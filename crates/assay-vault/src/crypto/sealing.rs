//! Sealing — KEK protection at rest.
//!
//! Plan 17 §S7. The master KEK protects every wrapped DEK in KV +
//! transit and every collection-key envelope. Phase 1 shipped a
//! `plaintext` placeholder where the KEK at rest IS the raw 32 bytes;
//! Phase 2 lands real sealing options:
//!
//! - **Shamir Secret Sharing init unseal** — split the 32-byte KEK into
//!   N shares; require K to reconstruct on boot. Pure-Rust crypto via
//!   the `sharks` crate (Galois-field-256 SSS, audited). Default for
//!   non-cloud installs. Phase 2 ships this in full.
//! - **Cloud KMS auto-unseal** — wrap the KEK with AWS KMS or GCP KMS
//!   and decrypt on boot. Phase 2 ships the trait shape; the actual
//!   sigv4 / JWT calls land in Phase 5 alongside the same primitives
//!   the AWS / GCP dynamic-creds providers need.
//! - **HSM via PKCS#11** — opt-in via `vault-sealing-hsm`. Reserved.
//!
//! Sealing changes the at-rest format of the KEK row but leaves the
//! [`crate::crypto::kek::KekHandle`] API stable — every consumer
//! continues to call `wrap_dek` / `unwrap_dek` against the unsealed
//! handle.

use crate::error::{Result, VaultError};

/// Sealing method recorded in `vault.kek_metadata.sealing_method`.
/// `Display` emits the wire-format string the column expects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SealingMethod {
    /// Phase-1 placeholder. Blob IS the raw 32 bytes. WARN-logged.
    Plaintext,
    /// Shamir Secret Sharing — KEK split into N shares; threshold K
    /// shares reconstruct on unseal. Phase 2 default for non-cloud.
    Shamir { threshold: u8, shares_count: u8 },
    /// AWS KMS auto-unseal — wraps KEK via AWS KMS Encrypt. Phase 2
    /// reserves the variant; Phase 5 wires the sigv4 path.
    KmsAws { region: String, key_id: String },
    /// GCP KMS auto-unseal — wraps KEK via Google Cloud KMS encrypt.
    /// Same Phase-5 schedule as AWS.
    KmsGcp {
        project: String,
        location: String,
        key_ring: String,
        key: String,
    },
}

impl SealingMethod {
    pub fn as_column(&self) -> &'static str {
        match self {
            Self::Plaintext => "plaintext",
            Self::Shamir { .. } => "shamir",
            Self::KmsAws { .. } => "kms-aws",
            Self::KmsGcp { .. } => "kms-gcp",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "plaintext" => Ok(Self::Plaintext),
            // Shamir + KMS variants need their parameters from
            // surrounding columns (share_threshold, share_count, KMS
            // config from engine.toml). The store layer hands the
            // populated variant in; this parser is for stand-alone
            // round-trip tests.
            other => Err(VaultError::Invalid(format!(
                "sealing_method '{other}' has parameters not encoded in the column"
            ))),
        }
    }
}

/// Shamir Secret Sharing helpers. Gated on the `vault-sealing-shamir`
/// feature so a slim build that only uses KMS auto-unseal doesn't pull
/// the `sharks` crate.
#[cfg(feature = "vault-sealing-shamir")]
pub mod shamir {
    use super::*;
    use crate::crypto::aead::KEY_LEN;

    /// One unseal share — the wire format an operator passes back to
    /// `unseal`. Internally it's the byte representation `sharks`
    /// produces.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Share(pub Vec<u8>);

    impl Share {
        pub fn from_bytes(b: Vec<u8>) -> Self {
            Self(b)
        }
        pub fn as_bytes(&self) -> &[u8] {
            &self.0
        }
    }

    /// Split a 32-byte KEK into `shares_count` Shamir shares; any
    /// `threshold` shares reconstruct it. Validates the threshold ≤
    /// shares_count and both ≥ 1 — passing 0 to `sharks` panics.
    pub fn split_kek(
        kek: &[u8; KEY_LEN],
        threshold: u8,
        shares_count: u8,
    ) -> Result<Vec<Share>> {
        if threshold == 0 || shares_count == 0 {
            return Err(VaultError::Invalid(
                "shamir threshold and shares_count must be ≥ 1".into(),
            ));
        }
        if threshold > shares_count {
            return Err(VaultError::Invalid(format!(
                "shamir threshold {threshold} exceeds shares_count {shares_count}"
            )));
        }
        let s = sharks::Sharks(threshold);
        let dealer = s.dealer(kek);
        let out = dealer
            .take(shares_count as usize)
            .map(|sh| Share(Vec::from(&sh)))
            .collect::<Vec<_>>();
        Ok(out)
    }

    /// Reconstruct a 32-byte KEK from `threshold` shares. Returns
    /// `Sealed` if fewer shares were provided; returns `Crypto` if the
    /// shares fail to reconstruct (corruption, mismatched threshold,
    /// shares from a different secret).
    pub fn combine_shares(threshold: u8, shares: &[Share]) -> Result<[u8; KEY_LEN]> {
        if shares.len() < threshold as usize {
            return Err(VaultError::Sealed);
        }
        let s = sharks::Sharks(threshold);
        let parsed = shares
            .iter()
            .map(|sh| {
                sharks::Share::try_from(sh.0.as_slice())
                    .map_err(|e| VaultError::Crypto(format!("bad share: {e}")))
            })
            .collect::<Result<Vec<_>>>()?;
        let secret = s
            .recover(&parsed)
            .map_err(|e| VaultError::Crypto(format!("shamir recover: {e}")))?;
        if secret.len() != KEY_LEN {
            return Err(VaultError::Crypto(format!(
                "recovered secret is {} bytes; expected {KEY_LEN}",
                secret.len()
            )));
        }
        let mut key = [0u8; KEY_LEN];
        key.copy_from_slice(&secret);
        Ok(key)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::crypto::aead::random_dek;

        #[test]
        fn split_then_combine_threshold() {
            let kek = random_dek();
            let shares = split_kek(&kek, 3, 5).unwrap();
            assert_eq!(shares.len(), 5);
            // Any 3 shares reconstruct.
            let recovered = combine_shares(3, &shares[..3]).unwrap();
            assert_eq!(recovered, kek);
            // A different 3.
            let recovered2 = combine_shares(3, &shares[2..5]).unwrap();
            assert_eq!(recovered2, kek);
        }

        #[test]
        fn fewer_than_threshold_is_sealed() {
            let kek = random_dek();
            let shares = split_kek(&kek, 3, 5).unwrap();
            let res = combine_shares(3, &shares[..2]);
            assert!(matches!(res, Err(VaultError::Sealed)));
        }

        #[test]
        fn invalid_params_rejected() {
            let kek = [0u8; 32];
            assert!(split_kek(&kek, 0, 3).is_err());
            assert!(split_kek(&kek, 3, 0).is_err());
            assert!(split_kek(&kek, 5, 3).is_err());
        }

        #[test]
        fn corrupt_share_fails_to_combine() {
            let kek = random_dek();
            let mut shares = split_kek(&kek, 3, 5).unwrap();
            // Flip a byte in one share.
            shares[0].0[5] ^= 0xff;
            // The combine may either reconstruct a wrong secret or fail
            // outright depending on where the corruption hits — either
            // way the result is NOT equal to the original.
            if let Ok(bad) = combine_shares(3, &shares[..3]) {
                assert_ne!(bad, kek);
            }
        }
    }
}

/// KMS auto-unseal trait. Phase 2 reserves the surface; Phase 5 lands
/// the AWS sigv4 + GCP JWT impls alongside the same primitives that
/// the dynamic-creds providers need.
///
/// Implementations wrap / unwrap the 32-byte KEK using a remote KMS
/// service. The wrapped blob lives in `vault.kek_metadata.sealed_blob`.
#[async_trait::async_trait]
pub trait KmsSeal: Send + Sync + 'static {
    /// Encrypt the raw KEK via the remote KMS. Returned bytes are
    /// stored verbatim in `sealed_blob` and decrypted on every boot.
    async fn wrap_kek(&self, raw: &[u8]) -> Result<Vec<u8>>;

    /// Decrypt the blob back to the raw 32-byte KEK.
    async fn unwrap_kek(&self, wrapped: &[u8]) -> Result<Vec<u8>>;

    /// Stable identifier for logs (e.g. "aws-kms:us-east-1:alias/vault").
    fn identifier(&self) -> String;
}

/// Backend-pluggable seal-state mutations.
///
/// Mirrors the [`crate::kv::KvStore`] / [`crate::transit::TransitStore`]
/// trait pattern so seal operations route through the same backend
/// abstraction. Engine boot wires `Arc<dyn SealStore>` into
/// [`crate::ctx::VaultCtx`] and the `/sys/*` handlers call it; the
/// handlers don't know whether they're talking to PG or SQLite.
#[async_trait::async_trait]
pub trait SealStore: Send + Sync + 'static {
    /// Generate a fresh KEK + Shamir split + persist a new
    /// `vault.kek_metadata` row. Returns the new kid plus the share
    /// bytes (one `Vec<u8>` per share). The shares are returned ONCE —
    /// the engine does not retain a copy. Operators MUST distribute
    /// and store them securely.
    async fn init_shamir(
        &self,
        threshold: u8,
        shares_count: u8,
    ) -> Result<(String, Vec<Vec<u8>>)>;

    /// Update the at-rest sealed flag for a kid. The runtime
    /// [`crate::crypto::seal_state::SealState`] is the source of truth
    /// for in-memory state; this is the audit/reboot signal.
    async fn set_sealed(&self, kid: &str, sealed: bool) -> Result<()>;
}
