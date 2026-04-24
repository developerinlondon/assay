//! HTTP server wiring.
//!
//! Builds an axum `Router` that composes the workflow API + dashboard
//! under one port. Auth middleware is intentionally omitted at Phase 3
//! — engine runs open per plan 12 rev 2 until Phase 8 wires `assay-auth`
//! modules in.

use axum::Router;
use std::sync::Arc;
use tracing::info;

use assay_domain::events::EngineEventBus;
use assay_workflow::events::WorkflowEventBus;
use assay_workflow::{WorkflowCtx, WorkflowStore};

use crate::state::EngineState;

/// Compose the full `axum::Router` for the engine.
///
/// The workflow crate returns a `Router` that already embeds its state,
/// and the dashboard crate returns a `Router<Arc<DashboardCtx>>` that we
/// `.with_state()` here. Both are merged into a single stateless `Router`
/// ready for `axum::serve`.
pub fn build_app<S: WorkflowStore + 'static>(state: EngineState<S>) -> Router {
    let workflow_router = assay_workflow::api::router(Arc::clone(&state.workflow));
    let dashboard_router =
        assay_dashboard::workflow_router().with_state(Arc::clone(&state.dashboard));
    workflow_router.merge(dashboard_router)
}

/// Bind a TCP listener on `bind_addr` and serve the composed app.
pub async fn serve<S: WorkflowStore + 'static>(
    bind_addr: &str,
    state: EngineState<S>,
) -> anyhow::Result<()> {
    let app = build_app(state);
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .map_err(|e| anyhow::anyhow!("bind {bind_addr}: {e}"))?;
    let actual = listener.local_addr()?;
    info!(target: "assay-engine", %actual, "listening");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Start a `WorkflowCtx` around the given store with Phase 3 defaults
/// (no auth, binary version stamp).
pub fn build_workflow_ctx<S: WorkflowStore + 'static>(store: S) -> Arc<WorkflowCtx<S>> {
    let ctx = WorkflowCtx::start(Arc::new(store))
        .with_auth_mode(assay_workflow::auth_mode::AuthMode::no_auth())
        .with_binary_version(env!("CARGO_PKG_VERSION"));
    Arc::new(ctx)
}

/// Start a `WorkflowCtx` with the engine-events bus wired in so SSE +
/// dispatch-wakeup can consume from it.
pub fn build_workflow_ctx_with_bus<S: WorkflowStore + 'static>(
    store: S,
    bus: Arc<dyn EngineEventBus>,
) -> Arc<WorkflowCtx<S>> {
    let ctx = WorkflowCtx::start(Arc::new(store))
        .with_auth_mode(assay_workflow::auth_mode::AuthMode::no_auth())
        .with_binary_version(env!("CARGO_PKG_VERSION"))
        .with_event_bus(WorkflowEventBus::new(bus));
    Arc::new(ctx)
}
