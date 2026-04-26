//! Composed vault context — the value engine state holds for the vault
//! module. Mirrors the shape of [`assay_auth::AuthCtx`] so engine boot
//! can compose them in parallel.
//!
//! Phase 0 keeps this trivial; the with-builder methods get added as
//! the per-feature stores land:
//!
//! - Phase 1: `kv: Arc<dyn KvStore>`, `transit: Arc<dyn TransitStore>`,
//!   `master_kek: KekHandle`
//! - Phase 2: `sealing: SealingState`, `audit_forwarders: Vec<...>`
//! - Phase 3: `collections: Arc<dyn CollectionStore>`,
//!   `personal_vaults: Arc<dyn PersonalVaultStore>`
//! - Phase 4: `biscuit_root: BiscuitRoot`, `share_revocations: ...`
//! - Phase 5: `dynamic_providers: Vec<Arc<dyn DynamicCredsProvider>>`

use std::sync::Arc;

/// Composes into the engine's central state struct via
/// `axum::extract::FromRef`. Cheap to clone — every field will be
/// `Arc`-shared by Phase 1+; the inner-Arc shape is in place now so
/// later commits add fields without changing the public clone cost.
#[derive(Clone, Default)]
pub struct VaultCtx {
    #[allow(dead_code)]
    inner: Arc<VaultCtxInner>,
}

#[derive(Default)]
struct VaultCtxInner {
    // Phase 0 placeholder — see crate-level docs for the per-phase
    // field set.
}

impl VaultCtx {
    /// Construct an empty context. Phase 0 has no state to wire; later
    /// phases add `with_*` builder methods following the AuthCtx
    /// pattern.
    pub fn new() -> Self {
        Self::default()
    }
}
