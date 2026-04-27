//! Personal vault — 1 per user, holds the user's X25519 public key
//! used for envelope-wrapping collection keys.
//!
//! Plan 17 §S4. Auto-created on signup; the assay-auth user-create
//! hook calls into this module's `ensure_vault`. Items in the personal
//! vault are encrypted client-side with the user's own symmetric key
//! (derived client-side from the master password / passkey-attested
//! material — Phase 3 doesn't choose the derivation; the server just
//! holds ciphertext + the user's pubkey).
//!
//! ## Trait split
//!
//! Mirrors the [`crate::kv::KvStore`] / [`crate::transit::TransitStore`]
//! / [`crate::crypto::sealing::SealStore`] pattern: pure-IO trait with
//! Pg / Sqlite impls plugged in at engine boot.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// One personal-vault row. The `public_key` is X25519 raw (32 bytes).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PersonalVault {
    pub id: String,
    pub owner_user: String,
    pub public_key: Vec<u8>,
    pub created_at: f64,
}

#[async_trait]
pub trait PersonalVaultStore: Send + Sync + 'static {
    /// Ensure a personal vault exists for `owner_user`. Idempotent —
    /// returns the existing row if one is present, otherwise creates
    /// it. The caller-supplied `public_key` is used only on first
    /// create; subsequent calls leave the existing pubkey alone (a
    /// pubkey rotation is a separate explicit operation).
    async fn ensure_vault(
        &self,
        id: &str,
        owner_user: &str,
        public_key: &[u8],
    ) -> Result<PersonalVault>;

    /// Read by owner.
    async fn get_by_owner(&self, owner_user: &str) -> Result<Option<PersonalVault>>;

    /// Read by id.
    async fn get_by_id(&self, id: &str) -> Result<Option<PersonalVault>>;

    /// Replace the user's public key. Used by an explicit pubkey-
    /// rotation flow (Phase 3 ships this as primitive; HTTP shipping
    /// in a later commit).
    async fn rotate_public_key(&self, owner_user: &str, new_public_key: &[u8])
        -> Result<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_vault_serde_round_trip() {
        let v = PersonalVault {
            id: "v1".into(),
            owner_user: "alice".into(),
            public_key: vec![1, 2, 3, 4],
            created_at: 1.0,
        };
        let json = serde_json::to_string(&v).unwrap();
        let back: PersonalVault = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, v.id);
        assert_eq!(back.owner_user, v.owner_user);
        assert_eq!(back.public_key, v.public_key);
    }
}
