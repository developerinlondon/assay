//! Activity scheduling and lookup endpoints.
//!
//! These endpoints are the public face of `Engine::schedule_activity` and
//! `Engine::get_activity` — workflows (running on a worker) call POST to
//! schedule the next activity, and the worker polls GET while waiting for
//! the result. Idempotency on `(workflow_id, seq)` makes it safe to retry.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::api::workflows::AppError;
use crate::api::AppState;
use crate::store::WorkflowStore;
use crate::types::{ScheduleActivityOpts, WorkflowActivity};

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> {
    Router::new()
        .route("/workflows/{id}/activities", post(schedule_activity))
        .route("/activities/{id}", get(get_activity))
}

#[derive(Deserialize, ToSchema)]
pub struct ScheduleActivityRequest {
    /// Activity name (the worker matches this to a registered handler)
    pub name: String,
    /// Sequence number relative to the workflow. Used for idempotency:
    /// scheduling the same `(workflow_id, seq)` twice is a no-op on the
    /// second call. Workflows assign sequence numbers in execution order.
    pub seq: i32,
    /// Task queue to route the activity to (workers poll a specific queue)
    pub task_queue: String,
    /// JSON-serialisable input passed to the activity handler
    pub input: Option<serde_json::Value>,
    /// Maximum attempts before the activity is marked `FAILED` (default 3)
    pub max_attempts: Option<i32>,
    /// Initial retry backoff in seconds (default 1.0)
    pub initial_interval_secs: Option<f64>,
    /// Exponential backoff coefficient (default 2.0)
    pub backoff_coefficient: Option<f64>,
    /// Total time the activity has to complete before being failed (default 300)
    pub start_to_close_secs: Option<f64>,
    /// If set, an activity that hasn't heartbeated within this window is auto-failed
    pub heartbeat_timeout_secs: Option<f64>,
}

#[utoipa::path(
    post, path = "/api/v1/workflows/{id}/activities",
    tag = "activities",
    params(("id" = String, Path, description = "Workflow ID")),
    request_body = ScheduleActivityRequest,
    responses(
        (status = 201, description = "Activity scheduled", body = WorkflowActivity),
    ),
)]
pub async fn schedule_activity<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(workflow_id): Path<String>,
    Json(req): Json<ScheduleActivityRequest>,
) -> Result<(axum::http::StatusCode, Json<WorkflowActivity>), AppError> {
    let input = req.input.map(|v| v.to_string());
    let opts = ScheduleActivityOpts {
        max_attempts: req.max_attempts,
        initial_interval_secs: req.initial_interval_secs,
        backoff_coefficient: req.backoff_coefficient,
        start_to_close_secs: req.start_to_close_secs,
        heartbeat_timeout_secs: req.heartbeat_timeout_secs,
    };
    let act = state
        .engine
        .schedule_activity(
            &workflow_id,
            req.seq,
            &req.name,
            input.as_deref(),
            &req.task_queue,
            opts,
        )
        .await?;
    Ok((axum::http::StatusCode::CREATED, Json(act)))
}

#[utoipa::path(
    get, path = "/api/v1/activities/{id}",
    tag = "activities",
    params(("id" = i64, Path, description = "Activity ID")),
    responses(
        (status = 200, description = "Activity record", body = WorkflowActivity),
        (status = 404, description = "Not found"),
    ),
)]
pub async fn get_activity<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(id): Path<i64>,
) -> Result<Json<WorkflowActivity>, AppError> {
    match state.engine.get_activity(id).await? {
        Some(a) => Ok(Json(a)),
        None => Err(AppError::NotFound(format!("activity {id} not found"))),
    }
}
