use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::api::workflows::AppError;
use crate::api::AppState;
use crate::store::WorkflowStore;
use crate::types::WorkflowWorker;

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> {
    Router::new()
        .route("/workers/register", post(register_worker))
        .route("/workers/heartbeat", post(worker_heartbeat))
        .route("/tasks/poll", post(poll_task))
        .route("/tasks/{id}/complete", post(complete_task))
        .route("/tasks/{id}/fail", post(fail_task))
        .route("/tasks/{id}/heartbeat", post(heartbeat_task))
}

#[derive(Deserialize)]
struct RegisterWorkerRequest {
    identity: String,
    queue: String,
    workflows: Option<Vec<String>>,
    activities: Option<Vec<String>>,
    #[serde(default = "default_concurrent")]
    max_concurrent_workflows: i32,
    #[serde(default = "default_concurrent")]
    max_concurrent_activities: i32,
}

fn default_concurrent() -> i32 {
    10
}

#[derive(Serialize)]
struct RegisterWorkerResponse {
    worker_id: String,
}

async fn register_worker<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Json(req): Json<RegisterWorkerRequest>,
) -> Result<Json<RegisterWorkerResponse>, AppError> {
    let now = timestamp_now();
    let worker_id = format!("w-{}", &uuid_short());

    let worker = WorkflowWorker {
        id: worker_id.clone(),
        identity: req.identity,
        task_queue: req.queue,
        workflows: req.workflows.map(|v| serde_json::to_string(&v).unwrap()),
        activities: req.activities.map(|v| serde_json::to_string(&v).unwrap()),
        max_concurrent_workflows: req.max_concurrent_workflows,
        max_concurrent_activities: req.max_concurrent_activities,
        active_tasks: 0,
        last_heartbeat: now,
        registered_at: now,
    };

    state.engine.register_worker(&worker).await?;
    Ok(Json(RegisterWorkerResponse { worker_id }))
}

#[derive(Deserialize)]
struct HeartbeatRequest {
    worker_id: String,
}

async fn worker_heartbeat<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Json(req): Json<HeartbeatRequest>,
) -> Result<axum::http::StatusCode, AppError> {
    state.engine.heartbeat_worker(&req.worker_id).await?;
    Ok(axum::http::StatusCode::OK)
}

#[derive(Deserialize)]
struct PollQuery {
    queue: String,
    worker_id: String,
}

async fn poll_task<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Json(req): Json<PollQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let activity = state
        .engine
        .claim_activity(&req.queue, &req.worker_id)
        .await?;

    match activity {
        Some(act) => Ok(Json(serde_json::to_value(act)?)),
        None => Ok(Json(serde_json::json!({ "task": null }))),
    }
}

#[derive(Deserialize)]
struct CompleteTaskBody {
    result: Option<serde_json::Value>,
}

async fn complete_task<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(id): Path<i64>,
    Json(body): Json<CompleteTaskBody>,
) -> Result<axum::http::StatusCode, AppError> {
    let result = body.result.map(|v| v.to_string());
    state
        .engine
        .complete_activity(id, result.as_deref(), None, false)
        .await?;
    Ok(axum::http::StatusCode::OK)
}

#[derive(Deserialize)]
struct FailTaskBody {
    error: String,
}

async fn fail_task<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(id): Path<i64>,
    Json(body): Json<FailTaskBody>,
) -> Result<axum::http::StatusCode, AppError> {
    state
        .engine
        .complete_activity(id, None, Some(&body.error), true)
        .await?;
    Ok(axum::http::StatusCode::OK)
}

#[derive(Deserialize)]
struct HeartbeatTaskBody {
    details: Option<String>,
}

async fn heartbeat_task<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(id): Path<i64>,
    Json(body): Json<HeartbeatTaskBody>,
) -> Result<axum::http::StatusCode, AppError> {
    state
        .engine
        .heartbeat_activity(id, body.details.as_deref())
        .await?;
    Ok(axum::http::StatusCode::OK)
}

fn timestamp_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

fn uuid_short() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut h);
    std::thread::current().id().hash(&mut h);
    format!("{:016x}", h.finish())
}
