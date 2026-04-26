//! Composed engine state.
//!
//! `EngineState<S>` bundles the per-module contexts. Phase 3 composed
//! workflow + dashboard. Phase 8 adds an optional `AuthCtx` field
//! (gated on the `auth` Cargo feature) and an `axum::FromRef` impl so
//! the auth router's handlers can extract `AuthCtx` directly via
//! `State<AuthCtx>`. The `AdminApiKeys` value flows through the same
//! seam so admin endpoints (`/admin/oidc/*`, `/admin/auth/*`) can
//! enforce the configured bearer-token allowlist.

use std::sync::Arc;

use assay_auth::AuthCtx;
use assay_dashboard::DashboardCtx;
#[cfg(feature = "vault")]
use assay_vault::VaultCtx;
use assay_workflow::{WorkflowCtx, WorkflowStore};

pub use assay_auth::state::AdminApiKeys;

use crate::config::EngineConfig;

#[derive(Clone)]
pub struct EngineState<S: WorkflowStore> {
    pub workflow: Arc<WorkflowCtx<S>>,
    pub dashboard: Arc<DashboardCtx>,
    /// Composed auth context — present iff the runtime
    /// `engine.modules.auth.enabled` row is TRUE. `axum::FromRef`
    /// extracts it for the auth router's handlers.
    pub auth: Option<AuthCtx>,
    /// Composed vault context — present iff the `vault` Cargo feature
    /// is on AND the runtime `engine.modules.vault.enabled` row is
    /// TRUE. Plan 17's v0.3.0 module — KV / transit / collections /
    /// share / sealing / dynamic creds / BW-compat all hang off this.
    #[cfg(feature = "vault")]
    pub vault: Option<VaultCtx>,
    /// Admin API keys for the `/admin/*` HTTP surface — checked by
    /// auth handlers via `axum::extract::FromRef<EngineState<S>>` so
    /// the same value flows from `engine.toml` through to per-request
    /// auth gating without a global static. Empty when no admin
    /// surface is configured.
    pub admin_api_keys: Arc<Vec<String>>,
    /// Names of modules attached/loaded during boot — surfaced through
    /// `/healthz` for ops visibility (which functional modules this
    /// engine instance has wired up).
    pub modules: Arc<Vec<String>>,
    /// This instance's row in `engine.instances`. Lets `/healthz` and
    /// future visibility endpoints identify which engine process is
    /// answering.
    pub instance_id: uuid::Uuid,
    /// `assay-engine` crate version. Returned in
    /// `/api/v1/engine/core/health` so external monitors can correlate
    /// health checks with deployments.
    pub engine_version: &'static str,
    /// Wall-clock seconds since the UNIX epoch when this engine process
    /// finished booting. Surfaced through `/api/v1/engine/core/info` so the
    /// engine console can display "uptime" without an extra DB lookup
    /// per request — matches the value the engine wrote to its
    /// `engine.instances` row at boot.
    pub started_at: f64,
    /// Parsed `engine.toml` snapshot — admin endpoints serialise this
    /// (with secrets redacted) so the engine console's "Config" pane
    /// shows the operator exactly what the running engine is using.
    /// `Arc` so cloning state per-request stays cheap.
    pub engine_config: Arc<EngineConfig>,
}

impl<S: WorkflowStore> axum::extract::FromRef<EngineState<S>> for AuthCtx {
    /// FromRef impl so auth handlers can extract the resolved AuthCtx
    /// via `State<AuthCtx>`. Panics when the engine binary mounted the
    /// auth router without composing an AuthCtx — a misconfiguration
    /// the engine boot path is responsible for preventing (the auth
    /// router is only mounted when `state.auth.is_some()`).
    fn from_ref(s: &EngineState<S>) -> Self {
        s.auth
            .clone()
            .expect("auth router mounted without an AuthCtx — engine boot bug")
    }
}

impl<S: WorkflowStore> axum::extract::FromRef<EngineState<S>> for AdminApiKeys {
    fn from_ref(s: &EngineState<S>) -> Self {
        AdminApiKeys(Arc::clone(&s.admin_api_keys))
    }
}

#[cfg(feature = "vault")]
impl<S: WorkflowStore> axum::extract::FromRef<EngineState<S>> for VaultCtx {
    /// FromRef impl so vault handlers (added Phase 1+) can extract the
    /// resolved VaultCtx via `State<VaultCtx>`. Panics when the vault
    /// router is mounted without composing a VaultCtx — same engine-
    /// boot-bug surface AuthCtx uses.
    fn from_ref(s: &EngineState<S>) -> Self {
        s.vault
            .clone()
            .expect("vault router mounted without a VaultCtx — engine boot bug")
    }
}
