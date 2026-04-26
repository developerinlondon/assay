//! Composed vault context — the value engine state holds for the vault
//! module. Mirrors the shape of [`assay_auth::AuthCtx`] so engine boot
//! can compose them in parallel.
//!
//! Phase 1 wires the master KEK + KV + transit services as type-erased
//! `Arc<dyn …>` handles. Subsequent phases add:
//!
//! - Phase 2: `sealing: SealingState`, `audit_forwarders: Vec<…>`
//! - Phase 3: `collections: Arc<dyn CollectionStore>`,
//!   `personal_vaults: Arc<dyn PersonalVaultStore>`
//! - Phase 4: `biscuit_root: BiscuitRoot`, `share_revocations: …`
//! - Phase 5: `dynamic_providers: Vec<Arc<dyn DynamicCredsProvider>>`

use std::sync::Arc;

use crate::crypto::seal_state::SealState;
use crate::crypto::sealing::SealingMethod;
use crate::crypto::KekHandle;

#[cfg(feature = "vault-kv")]
use crate::kv::{KvService, KvStore};
#[cfg(feature = "vault-transit")]
use crate::transit::{TransitService, TransitStore};

/// Composes into the engine's central state struct via
/// `axum::extract::FromRef`. Cheap to clone — every service is
/// `Arc`-shared underneath the type-erased trait object.
#[derive(Clone)]
pub struct VaultCtx {
    /// Master KEK handle. Always present so KV / transit services can
    /// hold a clone — but the live sealing state in `seal_state`
    /// gates whether the handle is "trusted active" or stale (sealed).
    /// Per-request handlers MUST consult `seal_state.require_unsealed()`
    /// before touching key material.
    pub kek: KekHandle,
    /// Runtime sealing state. Phase 2 introduces this; the engine boot
    /// path wires it from `vault.kek_metadata`. For first-boot /
    /// plaintext deployments it starts unsealed; for shamir-sealed
    /// installations it starts sealed and an operator must call
    /// `/sys/unseal` to bring it up.
    pub seal_state: SealState,
    /// Backend-pluggable [`crate::crypto::sealing::SealStore`] —
    /// /sys/init delegates init_shamir + set_sealed through this. Same
    /// dyn pattern as KV / transit; engine boot wires Pg or Sqlite at
    /// startup. None when no sealing backend is wired (tests + slim
    /// builds without sharks).
    pub seal_store: Option<DynSealStore>,
    #[cfg(feature = "vault-kv")]
    pub kv: Option<KvService<DynKvStore>>,
    #[cfg(feature = "vault-transit")]
    pub transit: Option<TransitService<DynTransitStore>>,
    /// Personal-vault store (Phase 3). Holds 1-per-user X25519 pubkey
    /// rows; the assay-auth user-create hook calls ensure_vault.
    #[cfg(feature = "vault-collections")]
    pub personal_vaults: Option<DynPersonalVaultStore>,
    /// Collection store (Phase 3) — shared E2E containers + member
    /// envelope tracking.
    #[cfg(feature = "vault-collections")]
    pub collections: Option<DynCollectionStore>,
    /// E2E-encrypted item store (Phase 3).
    #[cfg(feature = "vault-collections")]
    pub items: Option<DynItemStore>,
    /// Folders for visual organization (Phase 3, Bitwarden-compat).
    #[cfg(feature = "vault-collections")]
    pub folders: Option<DynFolderStore>,
}

impl Default for VaultCtx {
    fn default() -> Self {
        let kek = KekHandle::generate_ephemeral();
        let seal_state = SealState::unsealed(
            SealingMethod::Plaintext,
            kek.kid().to_string(),
            kek.clone(),
        );
        Self {
            kek,
            seal_state,
            seal_store: None,
            #[cfg(feature = "vault-kv")]
            kv: None,
            #[cfg(feature = "vault-transit")]
            transit: None,
            #[cfg(feature = "vault-collections")]
            personal_vaults: None,
            #[cfg(feature = "vault-collections")]
            collections: None,
            #[cfg(feature = "vault-collections")]
            items: None,
            #[cfg(feature = "vault-collections")]
            folders: None,
        }
    }
}

impl VaultCtx {
    /// Construct an empty context with a fresh ephemeral KEK. Useful
    /// for tests + boot paths that haven't loaded the persistent KEK
    /// yet (engine boot replaces this via [`Self::with_kek`]).
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct from an explicit KEK handle. Engine boot calls this
    /// after `crypto::kek_store::load_or_init_*` returns. Initialises
    /// the seal state to `unsealed` with method = Plaintext (Phase-1
    /// shape). For shamir installs use [`Self::with_sealed_shamir`].
    pub fn with_kek(mut self, kek: KekHandle) -> Self {
        let seal_state = SealState::unsealed(
            SealingMethod::Plaintext,
            kek.kid().to_string(),
            kek.clone(),
        );
        self.kek = kek;
        self.seal_state = seal_state;
        self
    }

    /// Phase-2 builder: vault starts sealed; operator must submit
    /// shares via `/sys/unseal` to bring it up. The KEK held on the
    /// ctx is a placeholder until then — handlers must check
    /// `seal_state.require_unsealed()` before using it.
    pub fn with_sealed_shamir(mut self, kid: String, threshold: u8, shares_count: u8) -> Self {
        self.seal_state = SealState::sealed_shamir(kid, threshold, shares_count);
        self
    }

    /// Replace the seal state explicitly — engine boot uses this when
    /// a unified loader builds the state from `vault.kek_metadata`.
    pub fn with_seal_state(mut self, seal_state: SealState) -> Self {
        self.seal_state = seal_state;
        self
    }

    #[cfg(feature = "vault-kv")]
    pub fn with_kv<S: KvStore + 'static>(mut self, store: S) -> Self {
        let store: DynKvStore = Arc::new(store);
        self.kv = Some(KvService::new(store, self.seal_state.clone()));
        self
    }

    #[cfg(feature = "vault-transit")]
    pub fn with_transit<S: TransitStore + 'static>(mut self, store: S) -> Self {
        let store: DynTransitStore = Arc::new(store);
        self.transit = Some(TransitService::new(store, self.seal_state.clone()));
        self
    }

    /// Wire a backend-pluggable [`crate::crypto::sealing::SealStore`].
    /// Engine boot calls this with PgSealStore / SqliteSealStore.
    pub fn with_seal_store<S: crate::crypto::sealing::SealStore + 'static>(
        mut self,
        store: S,
    ) -> Self {
        self.seal_store = Some(Arc::new(store));
        self
    }

    #[cfg(feature = "vault-collections")]
    pub fn with_personal_vaults<S: crate::personal_vault::PersonalVaultStore + 'static>(
        mut self,
        store: S,
    ) -> Self {
        self.personal_vaults = Some(Arc::new(store));
        self
    }

    #[cfg(feature = "vault-collections")]
    pub fn with_collections<S: crate::collections::CollectionStore + 'static>(
        mut self,
        store: S,
    ) -> Self {
        self.collections = Some(Arc::new(store));
        self
    }

    #[cfg(feature = "vault-collections")]
    pub fn with_items<S: crate::items::ItemStore + 'static>(mut self, store: S) -> Self {
        self.items = Some(Arc::new(store));
        self
    }

    #[cfg(feature = "vault-collections")]
    pub fn with_folders<S: crate::items::FolderStore + 'static>(mut self, store: S) -> Self {
        self.folders = Some(Arc::new(store));
        self
    }
}

/// Trait-object alias for the seal store — same shape as DynKvStore /
/// DynTransitStore.
pub type DynSealStore = Arc<dyn crate::crypto::sealing::SealStore>;

#[cfg(feature = "vault-collections")]
pub type DynPersonalVaultStore = Arc<dyn crate::personal_vault::PersonalVaultStore>;
#[cfg(feature = "vault-collections")]
pub type DynCollectionStore = Arc<dyn crate::collections::CollectionStore>;
#[cfg(feature = "vault-collections")]
pub type DynItemStore = Arc<dyn crate::items::ItemStore>;
#[cfg(feature = "vault-collections")]
pub type DynFolderStore = Arc<dyn crate::items::FolderStore>;

/// Type alias for the `KvService` carried in [`VaultCtx`]. The
/// trait-object indirection lets the engine choose PG or SQLite at
/// runtime without instantiating the whole context generic.
#[cfg(feature = "vault-kv")]
pub type DynKvStore = Arc<dyn KvStore>;

/// Trait-object alias for transit — same rationale.
#[cfg(feature = "vault-transit")]
pub type DynTransitStore = Arc<dyn TransitStore>;

// Forward `KvStore` / `TransitStore` through the `Arc<dyn …>` so the
// blanket service can use them. async_trait emits these for free for
// `Arc<dyn Trait>` because the trait is `dyn`-compatible.
#[cfg(feature = "vault-kv")]
#[async_trait::async_trait]
impl KvStore for DynKvStore {
    async fn put_row(
        &self,
        path: &str,
        ciphertext: &[u8],
        nonce: &[u8],
        wrapped_dek: &[u8],
        kek_kid: &str,
        custom_md: &serde_json::Value,
    ) -> crate::error::Result<i64> {
        (**self)
            .put_row(path, ciphertext, nonce, wrapped_dek, kek_kid, custom_md)
            .await
    }
    async fn get_row(&self, path: &str, version: i64) -> crate::error::Result<Option<crate::kv::KvRow>> {
        (**self).get_row(path, version).await
    }
    async fn get_latest_row(&self, path: &str) -> crate::error::Result<Option<crate::kv::KvRow>> {
        (**self).get_latest_row(path).await
    }
    async fn list_meta(&self, prefix: &str) -> crate::error::Result<Vec<crate::kv::KvMeta>> {
        (**self).list_meta(prefix).await
    }
    async fn read_meta(&self, path: &str) -> crate::error::Result<Option<crate::kv::KvMeta>> {
        (**self).read_meta(path).await
    }
    async fn soft_delete(&self, path: &str, version: i64, deleted_at: f64) -> crate::error::Result<bool> {
        (**self).soft_delete(path, version, deleted_at).await
    }
    async fn destroy(&self, path: &str, version: i64) -> crate::error::Result<bool> {
        (**self).destroy(path, version).await
    }
    async fn undelete(&self, path: &str, version: i64) -> crate::error::Result<bool> {
        (**self).undelete(path, version).await
    }
}

#[cfg(feature = "vault-transit")]
#[async_trait::async_trait]
impl TransitStore for DynTransitStore {
    async fn create_key(
        &self,
        name: &str,
        algo: &str,
        version_wrapped: &[u8],
        kek_kid: &str,
    ) -> crate::error::Result<()> {
        (**self).create_key(name, algo, version_wrapped, kek_kid).await
    }
    async fn get_key(&self, name: &str) -> crate::error::Result<Option<crate::transit::TransitKey>> {
        (**self).get_key(name).await
    }
    async fn get_version(
        &self,
        name: &str,
        version: i64,
    ) -> crate::error::Result<Option<crate::transit::TransitVersion>> {
        (**self).get_version(name, version).await
    }
    async fn get_latest_version(
        &self,
        name: &str,
    ) -> crate::error::Result<Option<crate::transit::TransitVersion>> {
        (**self).get_latest_version(name).await
    }
    async fn rotate(&self, name: &str, version_wrapped: &[u8], kek_kid: &str) -> crate::error::Result<i64> {
        (**self).rotate(name, version_wrapped, kek_kid).await
    }
    async fn list_keys(&self) -> crate::error::Result<Vec<crate::transit::TransitKey>> {
        (**self).list_keys().await
    }
}
