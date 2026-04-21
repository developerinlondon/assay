pub mod activities;
pub mod api_keys;
pub mod auth;
pub mod dashboard;
pub mod events;
pub mod namespaces;
pub mod openapi;
pub mod public;
pub mod queues;
pub mod schedules;
pub mod tasks;
pub mod whitelabel;
pub mod workers;
pub mod workflow_tasks;
pub mod workflows;

use std::sync::Arc;

use axum::middleware;
use axum::Router;
use tokio::sync::broadcast;
use tracing::info;

use crate::auth_mode::AuthMode;
use crate::ctx::{BroadcastEvent, EngineEvent, WorkflowCtx};
use crate::store::WorkflowStore;

/// Build the full API router.
///
/// Three tiers:
///   1. **Authenticated `/api/v1/*`** — workflows, schedules, namespaces,
///      activities, tasks, workers, queues, api-keys, events, meta/version.
///      Gated by `auth::auth_middleware` when an auth mode is enabled.
///   2. **Public `/api/v1/*`** — health, version. Always unauthenticated so
///      Kubernetes probes, load balancers, and third-party monitors can
///      reach them without a bearer token.
///   3. **Dashboard + OpenAPI** — HTML/JSON at the root. Always public.
pub fn router<S: WorkflowStore>(state: Arc<WorkflowCtx<S>>) -> Router {
    let needs_auth = state.auth_mode.is_enabled();

    let authed_api = Router::new()
        .nest("/api/v1", api_v1_router::<S>())
        .nest("/api/v1", events::router::<S>());

    let authed_api = if needs_auth {
        authed_api.layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            auth::auth_middleware::<S>,
        ))
    } else {
        authed_api
    };

    // Public /api/v1/* routes — outside the auth layer by construction.
    let public_api = Router::new().nest("/api/v1", public::router::<S>());

    let app = authed_api
        .merge(public_api)
        .merge(dashboard::router::<S>())
        .merge(openapi::router::<S>());

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
pub async fn serve(
    store: impl WorkflowStore + 'static,
    port: u16,
    auth_mode: AuthMode,
) -> anyhow::Result<()> {
    serve_with_version(store, port, auth_mode, None).await
}

/// Like `serve`, but lets the embedder (e.g. the `assay` binary) pass
/// its own semver so `/api/v1/version` reflects the binary users are
/// actually running instead of the internal `assay-workflow` crate
/// version. Without this, the dashboard would show a misleading
/// "engine crate" version to operators.
pub async fn serve_with_version(
    store: impl WorkflowStore + 'static,
    port: u16,
    auth_mode: AuthMode,
    binary_version: Option<&'static str>,
) -> anyhow::Result<()> {
    // The SSE channel that dashboard browsers subscribe to. The
    // engine pushes EngineEvents into the bridge below; the bridge
    // converts each one to a BroadcastEvent and forwards to every
    // connected dashboard.
    let (sse_tx, _) = broadcast::channel::<BroadcastEvent>(1024);
    let (engine_tx, mut engine_rx) = broadcast::channel::<EngineEvent>(1024);

    let store = Arc::new(store);
    let mut ctx = WorkflowCtx::start(store)
        .with_event_broadcaster(engine_tx)
        .with_auth_mode(auth_mode.clone())
        .with_sse_tx(sse_tx.clone());

    if let Some(v) = binary_version {
        ctx = ctx.with_binary_version(v);
    }

    {
        let sse_tx = sse_tx.clone();
        tokio::spawn(async move {
            while let Ok(evt) = engine_rx.recv().await {
                let _ = sse_tx.send(BroadcastEvent {
                    event_type: evt.event_type,
                    workflow_id: evt.workflow_id,
                    payload: Some(serde_json::json!({
                        "namespace": evt.namespace,
                    }).to_string()),
                });
            }
        });
    }

    let mode_desc = auth_mode.describe();
    let state = Arc::new(ctx);

    let app = router(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    info!("Workflow API listening on 0.0.0.0:{port} (auth: {mode_desc})");

    axum::serve(listener, app).await?;
    Ok(())
}
