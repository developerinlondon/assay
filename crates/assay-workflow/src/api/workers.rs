use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::api::workflows::AppError;
use crate::api::AppState;
use crate::store::WorkflowStore;

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> {
    Router::new()
        .route("/workers", get(list_workers))
        .route("/health", get(health_check))
}

async fn list_workers<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let workers = state.engine.list_workers().await?;
    let json: Vec<serde_json::Value> = workers
        .into_iter()
        .map(|w| serde_json::to_value(w).unwrap_or_default())
        .collect();
    Ok(Json(json))
}

async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "assay-workflow",
    }))
}
