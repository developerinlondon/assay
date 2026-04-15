use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::workflows::AppError;
use crate::api::AppState;
use crate::store::WorkflowStore;
use crate::types::WorkflowSchedule;

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> {
    Router::new()
        .route("/schedules", post(create_schedule).get(list_schedules))
        .route(
            "/schedules/{name}",
            get(get_schedule).delete(delete_schedule),
        )
}

#[derive(Deserialize)]
struct CreateScheduleRequest {
    name: String,
    workflow_type: String,
    cron_expr: String,
    input: Option<serde_json::Value>,
    #[serde(default = "default_queue")]
    task_queue: String,
    #[serde(default = "default_overlap")]
    overlap_policy: String,
}

fn default_queue() -> String {
    "default".to_string()
}

fn default_overlap() -> String {
    "skip".to_string()
}

async fn create_schedule<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Json(req): Json<CreateScheduleRequest>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), AppError> {
    let now = timestamp_now();

    let schedule = WorkflowSchedule {
        name: req.name.clone(),
        workflow_type: req.workflow_type,
        cron_expr: req.cron_expr,
        input: req.input.map(|v| v.to_string()),
        task_queue: req.task_queue,
        overlap_policy: req.overlap_policy,
        paused: false,
        last_run_at: None,
        next_run_at: None,
        last_workflow_id: None,
        created_at: now,
    };

    state.engine.create_schedule(&schedule).await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(serde_json::to_value(schedule)?),
    ))
}

async fn list_schedules<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let schedules = state.engine.list_schedules().await?;
    let json: Vec<serde_json::Value> = schedules
        .into_iter()
        .map(|s| serde_json::to_value(s).unwrap_or_default())
        .collect();
    Ok(Json(json))
}

async fn get_schedule<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let schedule = state
        .engine
        .get_schedule(&name)
        .await?
        .ok_or(AppError::NotFound(format!("schedule {name}")))?;

    Ok(Json(serde_json::to_value(schedule)?))
}

async fn delete_schedule<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(name): Path<String>,
) -> Result<axum::http::StatusCode, AppError> {
    let deleted = state.engine.delete_schedule(&name).await?;
    if deleted {
        Ok(axum::http::StatusCode::OK)
    } else {
        Err(AppError::NotFound(format!("schedule {name}")))
    }
}

fn timestamp_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
