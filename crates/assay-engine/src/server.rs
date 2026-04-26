//! HTTP server wiring — composes the workflow API + dashboard + auth
//! routers into one axum `Router`. URL surface:
//!
//! - `/auth/*`                   OIDC spec (discovery, authorize, token, …)
//! - `/api/v1/engine/core/*`     engine-core admin
//! - `/api/v1/engine/workflow/*` workflow API (auth-gated below)
//! - `/api/v1/engine/auth/*`     engine-internal auth + admin
//! - `/healthz`                  redirect to `/api/v1/engine/core/health`

use axum::Router;
use axum::response::Redirect;
use axum::routing::get;
use std::sync::Arc;
use tracing::info;

use assay_domain::events::EngineEventBus;
use assay_workflow::events::WorkflowEventBus;
use assay_workflow::{WorkflowCtx, WorkflowStore};

use crate::state::EngineState;

/// Always-public paths under `/api/v1/engine/workflow/*` that bypass
/// the engine's auth gate (k8s probes, version banners, OpenAPI docs).
const WORKFLOW_PUBLIC_PATHS: &[&str] = &[
    "/api/v1/engine/workflow/health",
    "/api/v1/engine/workflow/version",
    "/api/v1/engine/workflow/openapi.json",
    "/api/v1/engine/workflow/docs",
];

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
    // Workflow router carries no auth of its own — slice 2 lifted that
    // to the engine layer. When `auth` is on AND an `AuthCtx` is
    // composed, wrap the workflow router with the gate middleware that
    // enforces `workflow:<namespace>#access` via `assay_auth::gate`.
    let workflow_router = assay_workflow::api::router(Arc::clone(&state.workflow));
        let workflow_router = if state.auth.is_some() {
        workflow_router.layer(axum::middleware::from_fn_with_state(
            state.clone(),
            workflow_gate_middleware::<S>,
        ))
    } else {
        workflow_router
    };

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

/// Engine-side gate middleware for the workflow API.
///
/// Per plan-15 slice 2 the engine is the auth boundary for every
/// module — the workflow no longer carries its own auth middleware.
/// This wrapper:
///
/// 1. Lets the always-public paths through unconditionally
///    ([`WORKFLOW_PUBLIC_PATHS`] — health, version, openapi, docs).
/// 2. Reads `?namespace=<X>` from the query string (defaults to
///    `main` to match the workflow store's default namespace).
/// 3. Calls [`assay_auth::gate::require_role_for`] for
///    `workflow:<namespace>#access`. Admin api-key callers bypass as
///    break-glass; session/JWT callers go through Zanzibar.
///
/// On gate failure the request short-circuits with the gate's response
/// (401 Unauthorized or 403 Forbidden); on success the request flows
/// to the workflow handler with the caller already authorised.
async fn workflow_gate_middleware<S: WorkflowStore + Clone + 'static>(
    axum::extract::State(state): axum::extract::State<EngineState<S>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = request.uri().path();
    if WORKFLOW_PUBLIC_PATHS
        .iter()
        .any(|p| path == *p || path.starts_with(&format!("{p}/")))
    {
        return next.run(request).await;
    }

    // Auth is on but no `AuthCtx` was composed at boot — this can
    // happen for tests that build a no-auth engine. Fail closed; the
    // engine binary's boot path always composes an AuthCtx when the
    // auth module is enabled.
    let Some(auth) = state.auth.as_ref() else {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "error": "service_unavailable",
                "error_description": "auth not configured on this engine instance",
            })),
        )
            .into_response();
    };

    let namespace = request
        .uri()
        .query()
        .and_then(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .find(|(k, _)| k == "namespace")
                .map(|(_, v)| v.into_owned())
        })
        .unwrap_or_else(|| "main".to_string());

    let keys = crate::state::AdminApiKeys(Arc::clone(&state.admin_api_keys));
    let headers = request.headers().clone();
    if let Err(r) = assay_auth::gate::require_role_for(
        &headers,
        auth,
        &keys,
        "workflow",
        &namespace,
        "access",
    )
    .await
    {
        return *r;
    }

    next.run(request).await
}

use axum::response::IntoResponse;

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

/// Start a `WorkflowCtx` around the given store. Authentication +
/// authorization are no longer the workflow's concern — the engine
/// wraps the workflow router with a gate middleware ([`build_app`])
/// that handles all of that.
pub fn build_workflow_ctx<S: WorkflowStore + 'static>(store: S) -> Arc<WorkflowCtx<S>> {
    let ctx = WorkflowCtx::start(Arc::new(store)).with_binary_version(env!("CARGO_PKG_VERSION"));
    Arc::new(ctx)
}

/// Like [`build_workflow_ctx`] but also wires the engine-wide event
/// bus into the workflow context so SSE + the dispatch-wakeup loop see
/// state transitions.
pub fn build_workflow_ctx_with_bus<S: WorkflowStore + 'static>(
    store: S,
    bus: Arc<dyn EngineEventBus>,
) -> Arc<WorkflowCtx<S>> {
    let ctx = WorkflowCtx::start(Arc::new(store))
        .with_binary_version(env!("CARGO_PKG_VERSION"))
        .with_event_bus(WorkflowEventBus::new(bus));
    Arc::new(ctx)
}

