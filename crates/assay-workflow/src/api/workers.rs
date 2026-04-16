use std::sync::Arc;

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::workflows::AppError;
use crate::api::AppState;
use crate::store::WorkflowStore;

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> {
    Router::new()
        .route("/workers", get(list_workers))
        .route("/health", get(health_check))
}

#[derive(Deserialize)]
pub struct NsQuery {
    #[serde(default = "default_namespace")]
    namespace: String,
}

fn default_namespace() -> String {
    "main".to_string()
}

#[utoipa::path(
    get, path = "/api/v1/workers",
    tag = "workers",
    params(("namespace" = Option<String>, Query, description = "Namespace (default: main)")),
    responses(
        (status = 200, description = "List of active workers", body = Vec<WorkflowWorker>),
    ),
)]
pub async fn list_workers<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Query(q): Query<NsQuery>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let workers = state.engine.list_workers(&q.namespace).await?;
    let json: Vec<serde_json::Value> = workers
        .into_iter()
        .map(|w| serde_json::to_value(w).unwrap_or_default())
        .collect();
    Ok(Json(json))
}

#[utoipa::path(
    get, path = "/api/v1/health",
    tag = "workers",
    responses(
        (status = 200, description = "Engine health status"),
    ),
)]
pub async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "assay-workflow",
    }))
}

use crate::types::WorkflowWorker;
