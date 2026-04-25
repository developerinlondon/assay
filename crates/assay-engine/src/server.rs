//! HTTP server wiring.
//!
//! Builds an axum `Router` that composes the workflow API + dashboard
//! under one port. Plan-15 lays out the locked URL surface:
//!
//! - `/auth/*`                         → OIDC spec endpoints (well-known,
//!   authorize, token, userinfo, revoke, introspect, logout, federation)
//! - `/api/v1/engine/core/*`           → engine-core admin
//! - `/api/v1/engine/workflow/*`       → workflow API
//! - `/api/v1/engine/auth/*`           → engine-internal auth (login,
//!   logout, whoami, passkey, admin)
//! - `/healthz`                        → 1-line redirect to
//!   `/api/v1/engine/core/health` for k8s probes
//!
//! The auth router is split into two routers (spec + engine-internal)
//! and mounted at distinct paths; the workflow API now nests under
//! `/api/v1/engine/workflow/`.

use axum::response::Redirect;
use axum::routing::get;
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
/// ready for `axum::serve`. When the `auth` feature is on AND the
/// engine boot constructed an `AuthCtx`, the OIDC spec router (mounted
/// at `/auth/`) and the engine-internal auth router (mounted under
/// `/api/v1/engine/auth/`) join the composition.
pub fn build_app<S: WorkflowStore + Clone + 'static>(state: EngineState<S>) -> Router {
    let workflow_router = assay_workflow::api::router(Arc::clone(&state.workflow));
    let dashboard_router =
        assay_dashboard::workflow_router().with_state(Arc::clone(&state.dashboard));

    // `/healthz` is kept as a 1-line redirect to the new engine-core
    // health endpoint for backward-compatible k8s probes. The real
    // health response is served by the engine-core router under
    // `/api/v1/engine/core/health` (see `engine_api.rs`).
    let healthz = Router::new().route(
        "/healthz",
        get(|| async {
            Redirect::permanent("/api/v1/engine/core/health")
        }),
    );

    // Engine-core admin API + console SPA. Always present (engine-core
    // is always running, regardless of which functional modules are
    // enabled). The admin handlers require a configured api-key —
    // when `admin_api_keys` is empty every admin route returns 401, so
    // mounting unconditionally is safe for no-auth builds. The
    // engine-core router carries `/api/v1/engine/core/info` (public),
    // `/api/v1/engine/core/health`, `/api/v1/engine/core/active-modules`,
    // and the admin endpoints.
    let engine_api_router = crate::engine_api::router::<S>().with_state(state.clone());
    let engine_console_router = assay_dashboard::engine_router();

    #[cfg_attr(not(feature = "auth"), allow(unused_mut))]
    let mut app = workflow_router
        .merge(dashboard_router)
        .merge(healthz)
        .merge(engine_api_router)
        .merge(engine_console_router);

    // Mount the auth routers when AuthCtx is present. We bind state to
    // each router *before* nesting so the merged tree remains
    // `Router<()>` (every other sub-router has its state baked in
    // similarly). This avoids the axum requirement that all merged
    // routers share a common state parameter.
    //
    // The routers are generic over a parent state from which both
    // `AuthCtx` and `AdminApiKeys` are extractable via `FromRef`;
    // `EngineState<S>` implements both impls (see `state.rs`), so the
    // engine threads its full state in once and the auth handlers
    // pluck what they need.
    #[cfg(feature = "auth")]
    if state.auth.is_some() {
        // OIDC spec endpoints — mounted at `/auth/...`. Discovery doc,
        // JWKS, authorize/token/userinfo/revoke/introspect/logout,
        // federation upstream callbacks. Stable surface that downstream
        // OIDC clients depend on.
        let spec_router =
            assay_auth::oidc_spec_router::<EngineState<S>>().with_state(state.clone());
        app = app.nest("/auth", spec_router);

        // Engine-internal auth — login, logout (DELETE), whoami,
        // passkey ceremonies, admin (users/sessions/biscuit/jwks/
        // zanzibar/audit + OIDC clients/upstream CRUD). Mounted under
        // `/api/v1/engine/auth/...` so the operator-facing surface
        // sits beside the engine-core + workflow APIs.
        let engine_auth_router =
            assay_auth::engine_auth_router::<EngineState<S>>().with_state(state.clone());
        app = app.nest("/api/v1/engine/auth", engine_auth_router);

        // Mount the auth-console SPA assets at root (so the same
        // `/auth/...` path namespace serves both the OIDC spec and the
        // dashboard asset bundle — `/auth/console` for the SPA,
        // `/api/v1/engine/auth/admin/*` for the admin JSON API).
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

