pub mod activities;
pub mod auth;
pub mod dashboard;
pub mod events;
pub mod meta;
pub mod namespaces;
pub mod openapi;
pub mod queues;
pub mod schedules;
pub mod tasks;
pub mod workers;
pub mod workflow_tasks;
pub mod workflows;

use std::sync::Arc;

use axum::middleware;
use axum::Router;
use tokio::sync::broadcast;
use tracing::info;

use crate::api::auth::AuthMode;
use crate::engine::Engine;
use crate::store::WorkflowStore;

/// Shared state for all API handlers.
pub struct AppState<S: WorkflowStore> {
    pub engine: Arc<Engine<S>>,
    pub event_tx: broadcast::Sender<events::BroadcastEvent>,
    pub auth_mode: AuthMode,
    /// Version of the containing binary (e.g. the `assay-lua` CLI) — set
    /// by embedders so `/api/v1/version` reflects the user-facing
    /// binary, not this internal engine-crate version. Defaults to the
    /// `assay-workflow` crate version, which is what the dashboard
    /// falls back to when an embedder doesn't override.
    pub binary_version: Option<&'static str>,
}

/// Build the full API router.
pub fn router<S: WorkflowStore + 'static>(state: Arc<AppState<S>>) -> Router {
    let needs_auth = !matches!(state.auth_mode, AuthMode::NoAuth);

    let api = Router::new()
        .nest("/api/v1", api_v1_router())
        .nest("/api/v1", events::router());

    let api = if needs_auth {
        api.layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            auth::auth_middleware,
        ))
    } else {
        api
    };

    // Dashboard + OpenAPI docs (no auth)
    let app = api
        .merge(dashboard::router())
        .merge(openapi::router());

    app.with_state(state)
}

fn api_v1_router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> {
    Router::new()
        .merge(workflows::router())
        .merge(activities::router())
        .merge(workflow_tasks::router())
        .merge(tasks::router())
        .merge(schedules::router())
        .merge(workers::router())
        .merge(namespaces::router())
        .merge(queues::router())
        .merge(meta::router())
}

/// Start the HTTP server on the given port.
pub async fn serve<S: WorkflowStore + 'static>(
    engine: Engine<S>,
    port: u16,
    auth_mode: AuthMode,
) -> anyhow::Result<()> {
    serve_with_version(engine, port, auth_mode, None).await
}

/// Like `serve`, but lets the embedder (e.g. the `assay` binary) pass
/// its own semver so `/api/v1/version` reflects the binary users are
/// actually running instead of the internal `assay-workflow` crate
/// version. Without this, the dashboard would show a misleading
/// "engine crate" version to operators.
pub async fn serve_with_version<S: WorkflowStore + 'static>(
    engine: Engine<S>,
    port: u16,
    auth_mode: AuthMode,
    binary_version: Option<&'static str>,
) -> anyhow::Result<()> {
    let (event_tx, _) = broadcast::channel(1024);

    let mode_desc = match &auth_mode {
        AuthMode::NoAuth => "no-auth (open access)".to_string(),
        AuthMode::ApiKey => "api-key".to_string(),
        AuthMode::Jwt { issuer, .. } => format!("jwt (issuer: {issuer})"),
    };

    let state = Arc::new(AppState {
        engine: Arc::new(engine),
        event_tx,
        auth_mode,
        binary_version,
    });

    let app = router(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    info!("Workflow API listening on 0.0.0.0:{port} (auth: {mode_desc})");

    axum::serve(listener, app).await?;
    Ok(())
}
