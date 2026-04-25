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
pub fn build_app<S: WorkflowStore + Clone + 'static>(state: EngineState<S>) -> Router {
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

    // `/api/v1/modules` reports the active modules list — read by the
    // dashboard JS so auth panes surface only when the auth module is
    // actually enabled (matches the `engine.modules` row + `engine.toml`
    // `auto_enable_modules` knob).
    let modules_for_api = Arc::clone(&state.modules);
    let modules_api = Router::new().route(
        "/api/v1/modules",
        get(move || {
            let modules = Arc::clone(&modules_for_api);
            async move {
                Json(serde_json::json!({
                    "modules": &*modules,
                }))
            }
        }),
    );

    // Engine-core admin API + console SPA. Always present (engine-core
    // is always running, regardless of which functional modules are
    // enabled). The admin handlers require a configured api-key —
    // when `admin_api_keys` is empty every admin route returns 401, so
    // mounting unconditionally is safe for no-auth builds.
    let engine_api_router = crate::engine_api::router::<S>().with_state(state.clone());
    let engine_console_router = assay_dashboard::engine_router();

    #[cfg_attr(not(feature = "auth"), allow(unused_mut))]
    let mut app = workflow_router
        .merge(dashboard_router)
        .merge(healthz)
        .merge(modules_api)
        .merge(engine_api_router)
        .merge(engine_console_router);

    // Mount the auth router under `/auth` when AuthCtx is present. We
    // bind state to the auth router *before* nesting so the merged tree
    // remains `Router<()>` (every other sub-router has its state baked in
    // similarly). This avoids the axum requirement that all merged
    // routers share a common state parameter.
    //
    // The router is generic over a parent state from which both
    // `AuthCtx` and `AdminApiKeys` are extractable via `FromRef`;
    // `EngineState<S>` implements both impls (see `state.rs`), so the
    // engine threads its full state in once and the auth handlers
    // pluck what they need.
    #[cfg(feature = "auth")]
    if state.auth.is_some() {
        // Use EngineState<S> as the auth router's parent state. It impls
        // both `FromRef<EngineState<S>> for AuthCtx` and
        // `FromRef<EngineState<S>> for AdminApiKeys` (see state.rs), so
        // the admin handlers and the user-facing handlers share the same
        // state seam. EngineState<S>: Clone is satisfied because both
        // PostgresStore and SqliteStore derive Clone (their pools are
        // already Arc'd internally).
        let auth_router =
            assay_auth::router::<EngineState<S>>().with_state(state.clone());
        app = app.nest("/auth", auth_router);
        // Mount the auth-console SPA assets at root (so the same /auth/...
        // path namespace serves both the API and the asset bundle —
        // /auth/console for the SPA, /auth/admin/* for the JSON API).
        let asset_router = assay_dashboard::auth_router();
        app = app.merge(asset_router);
    }

    app
}

/// Bind a TCP listener on `bind_addr` and serve the composed app.
pub async fn serve<S: WorkflowStore + Clone + 'static>(
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
