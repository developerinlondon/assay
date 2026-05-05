//! Live in-memory sealing state for a running engine instance.
//!
//! Companion to [`crate::crypto::kek_store`] (which handles the
//! at-rest representation) and [`crate::crypto::sealing`] (which holds
//! the primitives). This module owns the runtime state machine:
//!
//! - When `sealed = true`, the active [`KekHandle`] is logically gone;
//!   every KV / transit / collection-key operation should refuse.
//! - When `sealed = false`, the handle in the unsealed slot is the
//!   active KEK and operations proceed.
//! - During an unseal ceremony the [`UnsealAccumulator`] collects
//!   submitted shares until the threshold is reached, then yields the
//!   reconstructed KEK.

use parking_lot::RwLock;
use std::sync::Arc;

use crate::crypto::kek::KekHandle;
use crate::crypto::sealing::SealingMethod;
use crate::error::{Result, VaultError};

/// Snapshot of the live sealing state. Cheap to clone.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SealStatus {
    pub method: SealingMethod,
    pub sealed: bool,
    /// kek_metadata.kid for the active row.
    pub kid: Option<String>,
    /// For Shamir: how many shares have been submitted toward the next
    /// unseal. Resets on successful unseal or `seal`.
    pub shares_progress: u8,
    pub share_threshold: Option<u8>,
    pub share_count: Option<u8>,
}

/// Mutable runtime state. Held by [`crate::ctx::VaultCtx`] behind an
/// `Arc<RwLock<…>>`.
#[non_exhaustive]
pub struct SealStateInner {
    pub method: SealingMethod,
    pub kid: Option<String>,
    /// `None` when sealed; `Some(handle)` when unsealed.
    pub handle: Option<KekHandle>,
    /// Active accumulator for Shamir-method unseal. `None` when sealed
    /// with non-Shamir or already unsealed.
    pub accumulator: Option<UnsealAccumulator>,
    pub share_threshold: Option<u8>,
    pub share_count: Option<u8>,
}

/// Cheap clonable wrapper.
#[derive(Clone)]
#[non_exhaustive]
pub struct SealState {
    inner: Arc<RwLock<SealStateInner>>,
}

impl SealState {
    /// Construct from a fully-resolved unsealed KEK. Used by Phase 1's
    /// plaintext-on-boot flow (the KEK row decrypts trivially) and by
    /// the post-unseal path on the Shamir flow.
    pub fn unsealed(method: SealingMethod, kid: String, handle: KekHandle) -> Self {
        Self {
            inner: Arc::new(RwLock::new(SealStateInner {
                method,
                kid: Some(kid),
                handle: Some(handle),
                accumulator: None,
                share_threshold: None,
                share_count: None,
            })),
        }
    }

    /// Construct in the sealed Shamir state — engine boot calls this
    /// when `kek_metadata.sealed = TRUE` and an operator must unseal.
    /// The `kid` parameter is the content-addressed identifier from
    /// `vault.kek_metadata`; submitted shares must reconstruct a key
    /// whose own kid matches, otherwise the submission is rejected as
    /// corrupt.
    pub fn sealed_shamir(kid: String, threshold: u8, shares_count: u8) -> Self {
        Self {
            inner: Arc::new(RwLock::new(SealStateInner {
                method: SealingMethod::Shamir {
                    threshold,
                    shares_count,
                },
                kid: Some(kid),
                handle: None,
                accumulator: Some(UnsealAccumulator::new(threshold)),
                share_threshold: Some(threshold),
                share_count: Some(shares_count),
            })),
        }
    }

    /// Drop the in-memory KEK; future ops fail with [`VaultError::Sealed`].
    /// For Shamir-method state, primes a fresh accumulator for the next
    /// unseal.
    pub fn seal(&self) -> Result<()> {
        let mut g = self.inner.write();
        g.handle = None;
        match &g.method {
            SealingMethod::Shamir { threshold, .. } => {
                g.accumulator = Some(UnsealAccumulator::new(*threshold));
            }
            _ => {
                g.accumulator = None;
            }
        }
        Ok(())
    }

    /// Snapshot the public state.
    pub fn status(&self) -> SealStatus {
        let g = self.inner.read();
        SealStatus {
            method: g.method.clone(),
            sealed: g.handle.is_none(),
            kid: g.kid.clone(),
            shares_progress: g.accumulator.as_ref().map(|a| a.len() as u8).unwrap_or(0),
            share_threshold: g.share_threshold,
            share_count: g.share_count,
        }
    }

    /// Borrow the unsealed KEK or surface [`VaultError::Sealed`].
    pub fn require_unsealed(&self) -> Result<KekHandle> {
        let g = self.inner.read();
        g.handle.clone().ok_or(VaultError::Sealed)
    }

    /// Submit one Shamir unseal share. Returns the new
    /// [`SealStatus`]; if the threshold was hit by this submission,
    /// the state transitions to unsealed and `status.sealed = false`.
    /// Pass the raw share bytes the operator received from `init`.
    #[cfg(feature = "vault-sealing-shamir")]
    pub fn submit_shamir_share(&self, share_bytes: Vec<u8>) -> Result<SealStatus> {
        use crate::crypto::sealing::shamir::{Share, combine_shares};

        let mut g = self.inner.write();
        if g.handle.is_some() {
            return Err(VaultError::Invalid("vault is already unsealed".into()));
        }
        // Snapshot the read-only fields under the mutable lock before
        // taking the &mut borrow on accumulator — the borrow checker
        // wants exactly one borrow of `g` outstanding at a time.
        let threshold = g
            .share_threshold
            .ok_or_else(|| VaultError::Invalid("Shamir threshold missing".into()))?;
        let kid = g.kid.clone().unwrap_or_default();
        let acc = g
            .accumulator
            .as_mut()
            .ok_or_else(|| VaultError::Invalid("no unseal ceremony in progress".into()))?;

        acc.push(Share::from_bytes(share_bytes));

        if (acc.len() as u8) >= threshold {
            // Try to combine; on failure the accumulator gets reset so
            // a fresh unseal ceremony can start.
            let key = match combine_shares(threshold, acc.shares()) {
                Ok(k) => k,
                Err(e) => {
                    *acc = UnsealAccumulator::new(threshold);
                    return Err(e);
                }
            };
            // Shamir's Secret Sharing has no integrity check — if a
            // share is corrupted but the math still succeeds, we get a
            // garbage 32-byte value. The kid is content-addressed (a
            // domain-separated SHA-256 truncation of the KEK), so we
            // compare the reconstructed kid against the stored one to
            // catch this silent-failure case.
            let recovered_kid = crate::crypto::kek::mint_kid(&key);
            if recovered_kid != kid {
                *acc = UnsealAccumulator::new(threshold);
                return Err(VaultError::Crypto(format!(
                    "shamir reconstructed an unexpected key (kid mismatch: \
                     stored '{kid}', recovered '{recovered_kid}'). Likely a \
                     corrupted or tampered share."
                )));
            }
            let handle = KekHandle::from_bytes(kid, key);
            g.handle = Some(handle);
            g.accumulator = None;
        }

        // Status snapshot under the same write lock so the caller sees
        // a consistent post-state.
        Ok(SealStatus {
            method: g.method.clone(),
            sealed: g.handle.is_none(),
            kid: g.kid.clone(),
            shares_progress: g.accumulator.as_ref().map(|a| a.len() as u8).unwrap_or(0),
            share_threshold: g.share_threshold,
            share_count: g.share_count,
        })
    }

    /// Replace the active KEK (for KEK rotation / KMS auto-unseal).
    pub fn set_unsealed(&self, kid: String, handle: KekHandle) {
        let mut g = self.inner.write();
        g.kid = Some(kid);
        g.handle = Some(handle);
        g.accumulator = None;
    }
}

/// Collected shares during a Shamir unseal ceremony.
#[non_exhaustive]
pub struct UnsealAccumulator {
    threshold: u8,
    #[cfg(feature = "vault-sealing-shamir")]
    shares: Vec<crate::crypto::sealing::shamir::Share>,
}

impl UnsealAccumulator {
    pub fn new(threshold: u8) -> Self {
        Self {
            threshold,
            #[cfg(feature = "vault-sealing-shamir")]
            shares: Vec::new(),
        }
    }

    pub fn threshold(&self) -> u8 {
        self.threshold
    }

    #[cfg(feature = "vault-sealing-shamir")]
    pub fn len(&self) -> usize {
        self.shares.len()
    }

    #[cfg(not(feature = "vault-sealing-shamir"))]
    pub fn len(&self) -> usize {
        0
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[cfg(feature = "vault-sealing-shamir")]
    pub fn push(&mut self, share: crate::crypto::sealing::shamir::Share) {
        self.shares.push(share);
    }

    #[cfg(feature = "vault-sealing-shamir")]
    pub fn shares(&self) -> &[crate::crypto::sealing::shamir::Share] {
        &self.shares
    }
}

#[cfg(test)]
#[cfg(feature = "vault-sealing-shamir")]
mod tests {
    use super::*;
    use crate::crypto::aead::random_dek;
    use crate::crypto::sealing::shamir::{Share, split_kek};

    #[test]
    fn unsealed_round_trip() {
        let kek = random_dek();
        let handle = KekHandle::from_bytes("kek-test", kek);
        let s = SealState::unsealed(SealingMethod::Plaintext, "kek-test".into(), handle);
        assert!(!s.status().sealed);
        let _h = s.require_unsealed().unwrap();
        s.seal().unwrap();
        assert!(s.status().sealed);
        assert!(matches!(s.require_unsealed(), Err(VaultError::Sealed)));
    }

    #[test]
    fn shamir_unseal_via_share_submission() {
        let kek = random_dek();
        let shares = split_kek(&kek, 3, 5).unwrap();
        let kid = crate::crypto::kek::mint_kid(&kek);
        let s = SealState::sealed_shamir(kid, 3, 5);
        assert!(s.status().sealed);

        // First two shares — still sealed.
        for sh in &shares[..2] {
            let st = s.submit_shamir_share(sh.as_bytes().to_vec()).unwrap();
            assert!(st.sealed);
        }
        assert_eq!(s.status().shares_progress, 2);

        // Third share trips the threshold.
        let st = s
            .submit_shamir_share(shares[2].as_bytes().to_vec())
            .unwrap();
        assert!(!st.sealed, "threshold submission must unseal");
        // Reconstructed KEK matches the original — proven by wrapping
        // a known DEK on each side and comparing the unwrap result.
        let h = s.require_unsealed().unwrap();
        let test_dek = random_dek();
        let original = KekHandle::from_bytes(crate::crypto::kek::mint_kid(&kek), kek);
        let wrapped_by_original = original.wrap_dek(&test_dek).unwrap();
        let recovered = h.unwrap_dek(&wrapped_by_original).unwrap();
        assert_eq!(recovered, test_dek);
    }

    #[test]
    fn submit_after_unsealed_errors() {
        let kek = random_dek();
        let h = KekHandle::from_bytes("k", kek);
        let s = SealState::unsealed(SealingMethod::Plaintext, "k".into(), h);
        let res = s.submit_shamir_share(vec![1, 2, 3]);
        assert!(matches!(res, Err(VaultError::Invalid(_))));
    }

    #[test]
    fn corrupt_share_resets_accumulator() {
        let kek = random_dek();
        let shares = split_kek(&kek, 3, 5).unwrap();
        let kid = crate::crypto::kek::mint_kid(&kek);
        let s = SealState::sealed_shamir(kid.clone(), 3, 5);

        // Two good shares.
        s.submit_shamir_share(shares[0].as_bytes().to_vec())
            .unwrap();
        s.submit_shamir_share(shares[1].as_bytes().to_vec())
            .unwrap();
        // Garbled third share — combine either fails outright or
        // reconstructs a bogus key whose kid doesn't match. Both paths
        // reset the accumulator and return an error.
        let mut bad = shares[2].as_bytes().to_vec();
        for b in &mut bad {
            *b ^= 0xff;
        }
        let res = s.submit_shamir_share(bad);
        assert!(
            res.is_err(),
            "corrupt share must either fail combine or fail kid validation"
        );
        // Accumulator was reset.
        assert!(s.status().sealed);
        assert_eq!(s.status().shares_progress, 0);
        // A fresh ceremony with the real shares should succeed.
        s.submit_shamir_share(shares[0].as_bytes().to_vec())
            .unwrap();
        s.submit_shamir_share(shares[1].as_bytes().to_vec())
            .unwrap();
        let st = s
            .submit_shamir_share(shares[2].as_bytes().to_vec())
            .unwrap();
        assert!(!st.sealed);
    }

    #[test]
    fn seal_clears_handle_and_resets_accumulator() {
        let kek = random_dek();
        let shares = split_kek(&kek, 3, 5).unwrap();
        let kid = crate::crypto::kek::mint_kid(&kek);
        let s = SealState::sealed_shamir(kid, 3, 5);
        for sh in &shares[..3] {
            s.submit_shamir_share(sh.as_bytes().to_vec()).unwrap();
        }
        assert!(!s.status().sealed);
        s.seal().unwrap();
        assert!(s.status().sealed);
        assert_eq!(s.status().shares_progress, 0);
    }

    /// Ensures the type-erased `Share` import doesn't get optimised
    /// away by the dead-code lint.
    #[test]
    fn share_bytes_round_trip() {
        let share = Share::from_bytes(vec![1, 2, 3]);
        assert_eq!(share.as_bytes(), &[1, 2, 3]);
    }
}
