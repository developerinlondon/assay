pub mod auth;
pub mod dashboard;
pub mod events;
pub mod openapi;
pub mod schedules;
pub mod tasks;
pub mod workers;
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
        .merge(tasks::router())
        .merge(schedules::router())
        .merge(workers::router())
}

/// Start the HTTP server on the given port.
pub async fn serve<S: WorkflowStore + 'static>(
    engine: Engine<S>,
    port: u16,
    auth_mode: AuthMode,
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
    });

    let app = router(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    info!("Workflow API listening on 0.0.0.0:{port} (auth: {mode_desc})");

    axum::serve(listener, app).await?;
    Ok(())
}
