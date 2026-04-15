pub mod events;
pub mod schedules;
pub mod tasks;
pub mod workers;
pub mod workflows;

use std::sync::Arc;

use axum::Router;
use tokio::sync::broadcast;
use tracing::info;

use crate::engine::Engine;
use crate::store::WorkflowStore;

/// Shared state for all API handlers.
pub struct AppState<S: WorkflowStore> {
    pub engine: Arc<Engine<S>>,
    pub event_tx: broadcast::Sender<events::BroadcastEvent>,
}

/// Build the full API router.
pub fn router<S: WorkflowStore + 'static>(state: Arc<AppState<S>>) -> Router {
    Router::new()
        .nest("/api/v1", api_v1_router())
        .nest("/api/v1", events::router())
        .with_state(state)
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
) -> anyhow::Result<()> {
    let (event_tx, _) = broadcast::channel(1024);

    let state = Arc::new(AppState {
        engine: Arc::new(engine),
        event_tx,
    });

    let app = router(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    info!("Workflow API listening on 0.0.0.0:{port}");

    axum::serve(listener, app).await?;
    Ok(())
}
