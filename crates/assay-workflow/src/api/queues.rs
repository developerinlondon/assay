use std::sync::Arc;

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::workflows::AppError;
use crate::api::AppState;
use crate::store::WorkflowStore;

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> {
    Router::new().route("/queues", get(get_queue_stats))
}

#[derive(Deserialize)]
pub struct NsQuery {
    #[serde(default = "default_namespace")]
    pub namespace: String,
}

fn default_namespace() -> String {
    "main".to_string()
}

#[utoipa::path(
    get, path = "/api/v1/queues",
    tag = "workers",
    params(("namespace" = Option<String>, Query, description = "Namespace (default: main)")),
    responses(
        (status = 200, description = "Queue statistics", body = Vec<QueueStats>),
    ),
)]
pub async fn get_queue_stats<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Query(q): Query<NsQuery>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let stats = state.engine.get_queue_stats(&q.namespace).await?;
    let json: Vec<serde_json::Value> = stats
        .into_iter()
        .map(|s| serde_json::to_value(s).unwrap_or_default())
        .collect();
    Ok(Json(json))
}

use crate::store::QueueStats;
