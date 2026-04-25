//! HTTP server wiring.
//!
//! Builds an axum `Router` that composes the workflow API + dashboard
//! under one port. Phase 8 adds the optional auth router (mounted at
//! `/auth`) when an `AuthCtx` is present in `EngineState` — gated by
//! the `auth` Cargo feature so a no-auth build compiles unchanged.

use axum::routing::get;
use axum::{Json, Router};
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
/// ready for `axum::serve`. When the `auth` feature is on AND the
/// engine boot constructed an `AuthCtx`, the auth router (mounted at
/// `/auth`) joins the composition.
pub fn build_app<S: WorkflowStore + 'static>(state: EngineState<S>) -> Router {
    let workflow_router = assay_workflow::api::router(Arc::clone(&state.workflow));
    let dashboard_router =
        assay_dashboard::workflow_router().with_state(Arc::clone(&state.dashboard));

    // Engine-level `/healthz` reports the modules attached at boot, this
    // instance's id, and the running engine version. Distinct from the
    // workflow module's `/api/v1/health` (which is a static OK probe).
    let modules = Arc::clone(&state.modules);
    let instance_id = state.instance_id;
    let engine_version = state.engine_version;
    let healthz = Router::new().route(
        "/healthz",
        get(move || {
            let modules = Arc::clone(&modules);
            async move {
                Json(serde_json::json!({
                    "status": "ok",
                    "engine_version": engine_version,
                    "instance_id": instance_id.to_string(),
                    "modules": &*modules,
                    // SQLite is single-instance and PG uses session-scoped
                    // pg_try_advisory_lock; both make leadership a runtime
                    // property. Surface it as `single_node` for SQLite (no
                    // election) and let dashboards keep the field stable.
                    "leader": true,
                }))
            }
        }),
    );

    #[cfg_attr(not(feature = "auth"), allow(unused_mut))]
    let mut app = workflow_router.merge(dashboard_router).merge(healthz);

    // Mount the auth router under `/auth` when AuthCtx is present. We
    // bind state to the auth router *before* nesting so the merged tree
    // remains `Router<()>` (every other sub-router has its state baked in
    // similarly). This avoids the axum requirement that all merged
    // routers share a common state parameter.
    #[cfg(feature = "auth")]
    if let Some(auth_ctx) = state.auth.clone() {
        let auth_router = assay_auth::router::<assay_auth::AuthCtx>().with_state(auth_ctx);
        app = app.nest("/auth", auth_router);
    }

    app
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
