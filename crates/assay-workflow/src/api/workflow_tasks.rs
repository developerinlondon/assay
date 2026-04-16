//! Workflow-task dispatch endpoints (Phase 9).
//!
//! A "workflow task" represents "this workflow has new events that need a
//! worker to run the workflow handler against." It's distinct from an
//! "activity task" (which runs concrete activity code). The dispatch loop:
//!
//! 1. The engine sets `needs_dispatch=true` on a workflow when something
//!    workflow-visible happens (started, activity completed, timer fired,
//!    signal arrived).
//! 2. A worker calls `POST /workflow-tasks/poll` to claim the next
//!    dispatchable workflow on its queue. Response carries the workflow
//!    id, type, input, and full event history for replay.
//! 3. The worker invokes the handler in a coroutine that yields commands
//!    (ScheduleActivity, CompleteWorkflow, FailWorkflow, etc.) instead of
//!    making side effects directly.
//! 4. The worker `POST /workflow-tasks/:id/commands` to submit the batch.
//!    The engine processes each command transactionally and releases the
//!    worker's claim on the workflow task.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::api::workflows::AppError;
use crate::api::AppState;
use crate::store::WorkflowStore;

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> {
    Router::new()
        .route("/workflow-tasks/poll", post(poll_workflow_task))
        .route("/workflow-tasks/{id}/commands", post(submit_commands))
}

#[derive(Deserialize, ToSchema)]
pub struct PollWorkflowTaskRequest {
    pub queue: String,
    pub worker_id: String,
}

/// Claim a dispatchable workflow on the requested queue. Response is `null`
/// when nothing is available (worker should sleep + retry).
#[utoipa::path(
    post, path = "/api/v1/workflow-tasks/poll",
    tag = "workflow-tasks",
    request_body = PollWorkflowTaskRequest,
    responses(
        (status = 200, description = "Workflow task or null"),
    ),
)]
pub async fn poll_workflow_task<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Json(req): Json<PollWorkflowTaskRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    match state
        .engine
        .claim_workflow_task(&req.queue, &req.worker_id)
        .await?
    {
        Some((wf, history)) => Ok(Json(serde_json::json!({
            "workflow_id": wf.id,
            "namespace": wf.namespace,
            "workflow_type": wf.workflow_type,
            "task_queue": wf.task_queue,
            "input": wf.input.as_deref().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
            "history": history.iter().map(|e| serde_json::json!({
                "seq": e.seq,
                "event_type": e.event_type,
                "payload": e.payload.as_deref().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
                "timestamp": e.timestamp,
            })).collect::<Vec<_>>(),
        }))),
        None => Ok(Json(serde_json::Value::Null)),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct SubmitCommandsRequest {
    pub worker_id: String,
    pub commands: Vec<serde_json::Value>,
}

/// Submit a batch of commands a worker produced from running the workflow
/// handler. Each command is processed transactionally (ScheduleActivity
/// inserts a row + appends ActivityScheduled, CompleteWorkflow flips the
/// status + appends WorkflowCompleted, etc.) and the worker's claim is
/// released on success.
#[utoipa::path(
    post, path = "/api/v1/workflow-tasks/{id}/commands",
    tag = "workflow-tasks",
    params(("id" = String, Path, description = "Workflow ID")),
    request_body = SubmitCommandsRequest,
    responses((status = 200, description = "Commands processed; lease released")),
)]
pub async fn submit_commands<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(workflow_id): Path<String>,
    Json(req): Json<SubmitCommandsRequest>,
) -> Result<axum::http::StatusCode, AppError> {
    state
        .engine
        .submit_workflow_commands(&workflow_id, &req.worker_id, &req.commands)
        .await?;
    Ok(axum::http::StatusCode::OK)
}
