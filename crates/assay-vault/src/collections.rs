//! Shared collections — Bitwarden-equivalent of "Organizations'
//! Collections". Plan 17 §S4 second slice.
//!
//! A collection is a shared container scoped to an org (or null org
//! for personal-team collections). Members are tracked via per-row
//! `wrapped_key` envelopes — the collection's symmetric key encrypted
//! to each member's X25519 pubkey via ECDH. The server never sees the
//! plaintext collection key; it only routes ciphertext + envelopes.
//!
//! ## Trait split
//!
//! Same shape as the rest of the vault traits. Pure-IO; impls in
//! `store::postgres` / `store::sqlite`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// One row in `vault.collections`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Collection {
    pub id: String,
    pub org_id: Option<String>,
    pub name: String,
    pub created_by: String,
    pub created_at: f64,
}

/// One row in `vault.collection_members`. `wrapped_key` is opaque to
/// the server — it's the collection's symmetric key encrypted to the
/// member's X25519 pubkey, produced client-side.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionMember {
    pub collection_id: String,
    pub user_id: String,
    pub wrapped_key: Vec<u8>,
    pub role: String,
    pub added_at: f64,
}

/// Roles. Plan 17 doesn't fully lock the role list yet; Phase 3 ships
/// the obvious three. Phase 7 BW-compat may extend.
pub mod roles {
    pub const VIEWER: &str = "viewer";
    pub const EDITOR: &str = "editor";
    pub const ADMIN: &str = "admin";
}

#[async_trait]
pub trait CollectionStore: Send + Sync + 'static {
    /// Create a new collection. The caller chooses the id (UUIDv7
    /// from the HTTP layer). Returns Conflict if id already exists.
    async fn create_collection(
        &self,
        id: &str,
        org_id: Option<&str>,
        name: &str,
        created_by: &str,
    ) -> Result<Collection>;

    async fn get_collection(&self, id: &str) -> Result<Option<Collection>>;

    /// List collections optionally scoped to an org. None = every row
    /// the caller is allowed to see (the access gate runs above this
    /// trait via Zanzibar — the trait itself returns rows verbatim).
    async fn list_collections(&self, org_id: Option<&str>) -> Result<Vec<Collection>>;

    /// Delete a collection + its members + its items (FK cascade).
    /// Returns true iff a row was removed.
    async fn delete_collection(&self, id: &str) -> Result<bool>;

    /// Add or replace a member's wrapped-key envelope. Idempotent in
    /// the sense that re-calling with the same (collection_id,
    /// user_id) updates `wrapped_key` + `role` rather than failing —
    /// matches the operational model where re-wraps happen on key
    /// rotation.
    async fn upsert_member(
        &self,
        collection_id: &str,
        user_id: &str,
        wrapped_key: &[u8],
        role: &str,
    ) -> Result<()>;

    async fn list_members(&self, collection_id: &str) -> Result<Vec<CollectionMember>>;

    async fn remove_member(&self, collection_id: &str, user_id: &str) -> Result<bool>;

    /// Whether the user is currently a member of the collection. The
    /// access gate consults this; the trait itself doesn't enforce.
    async fn is_member(&self, collection_id: &str, user_id: &str) -> Result<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_serde_round_trip() {
        let c = Collection {
            id: "c1".into(),
            org_id: Some("org-acme".into()),
            name: "Engineering".into(),
            created_by: "alice".into(),
            created_at: 1.0,
        };
        let s = serde_json::to_string(&c).unwrap();
        let back: Collection = serde_json::from_str(&s).unwrap();
        assert_eq!(back.id, c.id);
        assert_eq!(back.org_id, c.org_id);
    }

    #[test]
    fn role_constants_match_db_default() {
        // The schema defaults role to 'viewer'; ensure our constant
        // matches so the trait surface stays consistent with DDL.
        assert_eq!(roles::VIEWER, "viewer");
    }
}
