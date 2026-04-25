//! Composed engine state.
//!
//! `EngineState<S>` bundles the per-module contexts. Phase 3 composes
//! workflow + dashboard. Phase 8 adds auth (Arc<AuthCtx>) as an
//! additional field without touching Phase 3's wiring.

use std::sync::Arc;

use assay_dashboard::DashboardCtx;
use assay_workflow::{WorkflowCtx, WorkflowStore};

#[derive(Clone)]
pub struct EngineState<S: WorkflowStore> {
    pub workflow: Arc<WorkflowCtx<S>>,
    pub dashboard: Arc<DashboardCtx>,
    /// Names of modules attached/loaded during boot — surfaced through
    /// `/healthz` for ops visibility (which functional modules this
    /// engine instance has wired up).
    pub modules: Arc<Vec<String>>,
    /// This instance's row in `engine.instances`. Lets `/healthz` and
    /// future visibility endpoints identify which engine process is
    /// answering.
    pub instance_id: uuid::Uuid,
    /// `assay-engine` crate version. Returned in `/healthz` so external
    /// monitors can correlate health checks with deployments.
    pub engine_version: &'static str,
}
