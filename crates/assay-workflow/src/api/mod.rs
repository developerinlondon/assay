pub mod activities;
pub mod api_keys;
pub mod auth;
pub mod events;
pub mod namespaces;
pub mod openapi;
pub mod public;
pub mod queues;
pub mod schedules;
pub mod tasks;
pub mod workers;
pub mod workflow_tasks;
pub mod workflows;

use std::sync::Arc;

use assay_domain::events::EngineEventBus;
use axum::Router;
use axum::middleware;
use tracing::info;

use crate::auth_mode::AuthMode;
use crate::ctx::WorkflowCtx;
use crate::events::WorkflowEventBus;
use crate::store::WorkflowStore;

/// Build the full API router.
///
/// Three tiers (all under `/api/v1/engine/workflow/*` per plan 15):
///   1. **Authenticated** — workflows, schedules, namespaces,
///      activities, tasks, workers, queues, api-keys, events.
///      Gated by `auth::auth_middleware` when an auth mode is enabled.
///   2. **Public** — health, version. Always unauthenticated so
///      Kubernetes probes, load balancers, and third-party monitors can
///      reach them without a bearer token.
///   3. **OpenAPI** — `openapi.json` + `docs` HTML. Always public.
pub fn router<S: WorkflowStore>(state: Arc<WorkflowCtx<S>>) -> Router {
    let needs_auth = state.auth_mode.is_enabled();

    let authed_api = Router::new()
        .nest("/api/v1/engine/workflow", api_v1_router::<S>())
        .nest("/api/v1/engine/workflow", events::router::<S>());

    let authed_api = if needs_auth {
        authed_api.layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            auth::auth_middleware::<S>,
        ))
    } else {
        authed_api
    };

    // Public workflow routes — outside the auth layer by construction.
    let public_api = Router::new().nest("/api/v1/engine/workflow", public::router::<S>());

    let app = authed_api.merge(public_api).merge(openapi::router::<S>());

    app.with_state(state)
}

fn api_v1_router<S: WorkflowStore>() -> Router<Arc<WorkflowCtx<S>>> {
    Router::new()
        .merge(workflows::router::<S>())
        .merge(activities::router::<S>())
        .merge(workflow_tasks::router::<S>())
        .merge(tasks::router::<S>())
        .merge(schedules::router::<S>())
        .merge(workers::router::<S>())
        .merge(namespaces::router::<S>())
        .merge(queues::router::<S>())
        .merge(api_keys::router::<S>())
}

/// Start the HTTP server on the given port.
///
/// Legacy entry point without event-bus wiring. Kept so existing embedders
/// (e.g. tests / the `assay-lua` runtime harness) keep compiling; the engine
/// binary goes through `serve_with_bus` so dashboard SSE + dispatch-wakeup
/// loop have a live bus.
pub async fn serve(
    store: impl WorkflowStore + 'static,
    port: u16,
    auth_mode: AuthMode,
) -> anyhow::Result<()> {
    serve_inner(store, port, auth_mode, None, None).await
}

/// Like `serve`, but lets the embedder pass its own semver so
/// `/api/v1/engine/workflow/version` reflects the binary users are actually running.
pub async fn serve_with_version(
    store: impl WorkflowStore + 'static,
    port: u16,
    auth_mode: AuthMode,
    binary_version: Option<&'static str>,
) -> anyhow::Result<()> {
    serve_inner(store, port, auth_mode, binary_version, None).await
}

/// Preferred entry point for the `assay-engine` binary. Wires the
/// engine-wide `EngineEventBus` into the workflow context so emits
/// from state-mutating methods reach the SSE stream + dispatch-wakeup
/// loop.
pub async fn serve_with_bus(
    store: impl WorkflowStore + 'static,
    bus: Arc<dyn EngineEventBus>,
    port: u16,
    auth_mode: AuthMode,
    binary_version: Option<&'static str>,
) -> anyhow::Result<()> {
    serve_inner(store, port, auth_mode, binary_version, Some(bus)).await
}

async fn serve_inner(
    store: impl WorkflowStore + 'static,
    port: u16,
    auth_mode: AuthMode,
    binary_version: Option<&'static str>,
    bus: Option<Arc<dyn EngineEventBus>>,
) -> anyhow::Result<()> {
    let store = Arc::new(store);
    let mut ctx = WorkflowCtx::start(store).with_auth_mode(auth_mode.clone());
    if let Some(v) = binary_version {
        ctx = ctx.with_binary_version(v);
    }
    if let Some(b) = bus {
        ctx = ctx.with_event_bus(WorkflowEventBus::new(b));
    }

    let mode_desc = auth_mode.describe();
    let state = Arc::new(ctx);

    let app = router(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    info!("Workflow API listening on 0.0.0.0:{port} (auth: {mode_desc})");

    axum::serve(listener, app).await?;
    Ok(())
}
