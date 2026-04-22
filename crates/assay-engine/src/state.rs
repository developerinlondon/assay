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
}
