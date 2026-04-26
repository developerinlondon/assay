//! E2E-encrypted items + folders inside a personal vault or collection.
//!
//! Plan 17 §S4 final slice. Both items and folders attach to exactly
//! one parent — either a personal vault or a collection — and the
//! schema's CHECK constraint enforces the XOR.
//!
//! Items are encrypted client-side with the parent's symmetric key
//! (the personal-vault key or the collection key). The server only
//! sees ciphertext + nonce + minimal metadata (item_type, name) used
//! for indexing. Folders are pure visual organization (Bitwarden-
//! compat); they're not an access boundary.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Which container the item / folder belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parent<'a> {
    /// Personal vault, identified by its `vault.vaults.id`.
    Vault(&'a str),
    /// Shared collection, identified by `vault.collections.id`.
    Collection(&'a str),
}

impl<'a> Parent<'a> {
    pub fn vault_id(&self) -> Option<&'a str> {
        match self {
            Self::Vault(id) => Some(id),
            _ => None,
        }
    }
    pub fn collection_id(&self) -> Option<&'a str> {
        match self {
            Self::Collection(id) => Some(id),
            _ => None,
        }
    }
}

/// One row in `vault.items`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub vault_id: Option<String>,
    pub collection_id: Option<String>,
    pub folder_id: Option<String>,
    pub item_type: String,
    pub name: String,
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub created_at: f64,
    pub updated_at: f64,
}

/// One row in `vault.folders`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub vault_id: Option<String>,
    pub collection_id: Option<String>,
    pub parent_id: Option<String>,
    pub name: String,
    pub created_at: f64,
}

#[async_trait]
pub trait ItemStore: Send + Sync + 'static {
    /// Create one item attached to either a vault or a collection.
    /// Caller picks the id (UUIDv7). Optional `folder_id` for visual
    /// organization. Returns Conflict on duplicate id.
    async fn create_item(
        &self,
        id: &str,
        parent: Parent<'_>,
        folder_id: Option<&str>,
        item_type: &str,
        name: &str,
        ciphertext: &[u8],
        nonce: &[u8],
    ) -> Result<Item>;

    async fn get_item(&self, id: &str) -> Result<Option<Item>>;

    /// List every item under a parent, oldest first. Phase 3 returns
    /// rows verbatim; access gating is the HTTP layer's job.
    async fn list_items(&self, parent: Parent<'_>) -> Result<Vec<Item>>;

    /// Update an existing item's encrypted payload + folder placement.
    /// `name` and `item_type` are also updateable to support BW-style
    /// rename. Returns true iff the row was found and updated.
    async fn update_item(
        &self,
        id: &str,
        item_type: &str,
        name: &str,
        ciphertext: &[u8],
        nonce: &[u8],
        folder_id: Option<&str>,
    ) -> Result<bool>;

    /// Delete an item. Returns true iff a row was removed.
    async fn delete_item(&self, id: &str) -> Result<bool>;
}

#[async_trait]
pub trait FolderStore: Send + Sync + 'static {
    async fn create_folder(
        &self,
        id: &str,
        parent: Parent<'_>,
        parent_folder_id: Option<&str>,
        name: &str,
    ) -> Result<Folder>;

    async fn get_folder(&self, id: &str) -> Result<Option<Folder>>;

    async fn list_folders(&self, parent: Parent<'_>) -> Result<Vec<Folder>>;

    async fn rename_folder(&self, id: &str, name: &str) -> Result<bool>;

    /// Delete a folder. Items pointing at this folder remain in the
    /// parent container but their `folder_id` becomes dangling — the
    /// HTTP layer SHOULD null those references first; the storage
    /// layer is intentionally permissive (the schema doesn't FK
    /// folder_id, just keeps it as a TEXT pointer).
    async fn delete_folder(&self, id: &str) -> Result<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_extracts_one_id() {
        let v = Parent::Vault("v1");
        assert_eq!(v.vault_id(), Some("v1"));
        assert_eq!(v.collection_id(), None);
        let c = Parent::Collection("c1");
        assert_eq!(c.vault_id(), None);
        assert_eq!(c.collection_id(), Some("c1"));
    }

    #[test]
    fn item_serde_round_trip() {
        let i = Item {
            id: "i1".into(),
            vault_id: Some("v1".into()),
            collection_id: None,
            folder_id: None,
            item_type: "login".into(),
            name: "github".into(),
            ciphertext: vec![1, 2, 3],
            nonce: vec![4, 5, 6],
            created_at: 1.0,
            updated_at: 1.0,
        };
        let s = serde_json::to_string(&i).unwrap();
        let back: Item = serde_json::from_str(&s).unwrap();
        assert_eq!(back.id, i.id);
        assert_eq!(back.vault_id, i.vault_id);
        assert_eq!(back.ciphertext, i.ciphertext);
    }
}
