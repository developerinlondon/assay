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
    /// Master KEK handle. Always present; engine boot loads it from
    /// `vault.kek_metadata` (or seeds a fresh one on first boot).
    pub kek: KekHandle,
    #[cfg(feature = "vault-kv")]
    pub kv: Option<KvService<DynKvStore>>,
    #[cfg(feature = "vault-transit")]
    pub transit: Option<TransitService<DynTransitStore>>,
}

impl Default for VaultCtx {
    fn default() -> Self {
        Self {
            kek: KekHandle::generate_ephemeral(),
            #[cfg(feature = "vault-kv")]
            kv: None,
            #[cfg(feature = "vault-transit")]
            transit: None,
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
    /// after `crypto::kek_store::load_or_init_*` returns.
    pub fn with_kek(mut self, kek: KekHandle) -> Self {
        self.kek = kek;
        self
    }

    #[cfg(feature = "vault-kv")]
    pub fn with_kv<S: KvStore + 'static>(mut self, store: S) -> Self {
        let store: DynKvStore = Arc::new(store);
        self.kv = Some(KvService::new(store, self.kek.clone()));
        self
    }

    #[cfg(feature = "vault-transit")]
    pub fn with_transit<S: TransitStore + 'static>(mut self, store: S) -> Self {
        let store: DynTransitStore = Arc::new(store);
        self.transit = Some(TransitService::new(store, self.kek.clone()));
        self
    }
}

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
