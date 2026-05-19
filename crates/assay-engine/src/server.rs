//! HTTP server wiring — composes the workflow API + dashboard + auth
//! routers into one axum `Router`. URL surface:
//!
//! - `/auth/*`                   OIDC spec (discovery, authorize, token, …)
//! - `/api/v1/engine/core/*`     engine-core admin (admin-bearer-gated)
//! - `/api/v1/engine/workflow/*` workflow API (admin-bearer-gated)
//! - `/api/v1/engine/auth/*`     engine-internal auth + admin
//! - `/api/v1/vault/*`           vault module (admin-bearer-gated)
//! - `/healthz`                  redirect to `/api/v1/engine/core/health`
//!
//! Per the decoupled-modules architecture: each module accepts ONLY
//! an admin bearer token at its HTTP boundary. Per-user authentication
//! and policy decisions live upstream of the engine — typically in a
//! dashboard / BFF / API gateway that validates the user session, asks
//! zanzibar if they're allowed, and then forwards the call to the
//! engine using its own admin bearer. The engine itself does not
//! resolve sessions or check zanzibar at request time.
//!
//! Share-redeem (`GET /api/v1/vault/share/{token}`) is the one route
//! that bypasses admin bearer — the biscuit token in the URL is its
//! own auth, verified inside the handler.

use axum::Router;
use axum::response::Redirect;
use axum::routing::get;
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
    // Workflow router carries no auth of its own. The engine wraps it
    // with the admin-bearer middleware — that's the entire engine-side
    // authn surface. Per-user identity + policy live upstream in a
    // dashboard / BFF that calls this engine with its own admin bearer.
    let workflow_router = assay_workflow::api::router(Arc::clone(&state.workflow)).layer(
        axum::middleware::from_fn_with_state(state.clone(), admin_bearer_middleware::<S>),
    );

    // `/healthz` is kept as a 1-line redirect to the new engine-core
    // health endpoint for backward-compatible k8s probes. The real
    // health response is served by the engine-core router under
    // `/api/v1/engine/core/health` (see `engine_api.rs`).
    let healthz = Router::new().route(
        "/healthz",
        get(|| async { Redirect::permanent("/api/v1/engine/core/health") }),
    );

    // Engine-core admin API. The handlers require an admin api-key
    // bearer; when `admin_api_keys` is empty every admin route returns
    // 401, so mounting unconditionally is safe for no-auth builds.
    let engine_api_router = crate::engine_api::router::<S>().with_state(state.clone());

    // Operator dashboards (vault / workflow / engine-core console SPAs)
    // are NOT mounted by the engine — the decoupled-modules architecture
    // puts operator UX in the sysops dashboard (gondor). The engine
    // serves API only. /auth/login is the one browser-facing exception
    // (mounted below) — it's a protocol requirement for OIDC
    // authorization-code flow.
    let mut app = workflow_router.merge(healthz).merge(engine_api_router);

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

        // `/auth/login` — protocol surface for OIDC authorization-code
        // redirects. Browser-facing by design. The login pages and
        // their assets are the ONLY dashboard assets the engine still
        // hosts; everything else (operator SPAs, admin consoles) lives
        // in sysops/gondor.
        let login_router = assay_dashboard::auth_router();
        app = app.merge(login_router);
    }

    // Vault module — plan 17 / v0.3.0. Mounted under /api/v1/vault when
    // both the Cargo feature is on AND a VaultCtx was composed at boot
    // (i.e. engine.modules.vault.enabled was TRUE). Phase 1 routes are
    // admin-key-gated; Phase 3+ adds biscuit-share and Phase 7 the
    // BW-compat shim's per-user session auth.
    #[cfg(feature = "vault")]
    if state.vault.is_some() {
        // Vault router carries no per-handler auth. Engine wraps it
        // with admin-bearer middleware; share-redeem
        // (`GET /share/{token}`) is the only public route — the handler
        // verifies the biscuit token in the URL itself, so it must
        // bypass admin-bearer. That bypass is implemented inside
        // `admin_bearer_middleware` via the share-path predicate.
        let vault = assay_vault::router::vault_router::<EngineState<S>>()
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                admin_bearer_middleware::<S>,
            ))
            .with_state(state.clone());
        app = app.nest("/api/v1/vault", vault);
    }

    // BW-compat shim (Phase 7). Stock BW mobile / browser / CLI
    // clients hardcode /identity/* and /api/* — mount the compat
    // router at root so those clients work without a reverse-proxy
    // rewrite. Only reachable when both vault + bitwarden-compat
    // features are on AND VaultCtx + AuthCtx are composed.
    #[cfg(all(feature = "vault", feature = "vault-bitwarden-compat"))]
    if state.vault.is_some() && state.auth.is_some() {
        let bw =
            assay_vault::bitwarden_compat::router::<EngineState<S>>().with_state(state.clone());
        app = app.merge(bw);
    }

    app
}

/// Resource-server middleware applied to every engine module router.
/// Accepts EITHER the operator admin api-key (service-to-service /
/// break-glass) OR a JWT from a configured trusted issuer (per-user
/// resource-server pattern). No session, no zanzibar — policy lives
/// upstream.
///
/// The one bypass: vault share-redeem (`GET /api/v1/vault/share/{token}`,
/// excluding `/share/revoke`) — the biscuit token in the URL is its
/// own authentication.
async fn admin_bearer_middleware<S: WorkflowStore + Clone + 'static>(
    axum::extract::State(state): axum::extract::State<EngineState<S>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = request.uri().path();
    if path.starts_with("/api/v1/vault/share/") && path != "/api/v1/vault/share/revoke" {
        return next.run(request).await;
    }
    let keys = crate::state::AdminApiKeys(Arc::clone(&state.admin_api_keys));
    // If auth is configured, accept admin bearer OR trusted JWT.
    // If auth is not configured at all (no AuthCtx), only the admin
    // bearer path is available — fall back to the strict check.
    let outcome = match state.auth.as_ref() {
        Some(auth) => assay_auth::gate::require_admin_or_jwt(request.headers(), auth, &keys)
            .await
            .map(|_| ()),
        None => assay_auth::gate::require_admin_bearer(request.headers(), &keys),
    };
    if let Err(r) = outcome {
        return *r;
    }
    next.run(request).await
}

/// Bind a TCP listener on `bind_addr` and serve the composed app.
///
/// Convenience wrapper that composes [`EngineState`] into an
/// [`axum::Router`] via [`build_app`] and hands off to
/// [`bind_and_serve`]. Used by the standalone `assay-engine` binary.
pub async fn serve<S: WorkflowStore + Clone + 'static>(
    bind_addr: &str,
    state: EngineState<S>,
) -> anyhow::Result<()> {
    let app = build_app(state);
    bind_and_serve(bind_addr, app).await
}

/// Bind a TCP listener on `bind_addr` and serve a pre-built
/// [`axum::Router`].
///
/// Used by [`crate::run`] (after `embedded::build` returns the
/// composed router) and by downstream embedders who want to add
/// their own middleware / merge with their own router before
/// serving.
pub async fn bind_and_serve(bind_addr: &str, app: axum::Router) -> anyhow::Result<()> {
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
