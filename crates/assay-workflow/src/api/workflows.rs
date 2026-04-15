use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::api::AppState;
use crate::store::WorkflowStore;
use crate::types::WorkflowStatus;

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> {
    Router::new()
        .route("/workflows", post(start_workflow).get(list_workflows))
        .route("/workflows/{id}", get(describe_workflow))
        .route("/workflows/{id}/events", get(get_events))
        .route("/workflows/{id}/signal/{name}", post(send_signal))
        .route("/workflows/{id}/cancel", post(cancel_workflow))
        .route("/workflows/{id}/terminate", post(terminate_workflow))
}

#[derive(Deserialize)]
struct StartWorkflowRequest {
    workflow_type: String,
    workflow_id: String,
    input: Option<serde_json::Value>,
    #[serde(default = "default_queue")]
    task_queue: String,
}

fn default_queue() -> String {
    "default".to_string()
}

#[derive(Serialize)]
struct WorkflowResponse {
    workflow_id: String,
    run_id: String,
    status: String,
}

async fn start_workflow<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Json(req): Json<StartWorkflowRequest>,
) -> Result<(axum::http::StatusCode, Json<WorkflowResponse>), AppError> {
    let input = req.input.map(|v| v.to_string());
    let wf = state
        .engine
        .start_workflow(
            &req.workflow_type,
            &req.workflow_id,
            input.as_deref(),
            &req.task_queue,
        )
        .await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(WorkflowResponse {
            workflow_id: wf.id,
            run_id: wf.run_id,
            status: wf.status,
        }),
    ))
}

#[derive(Deserialize)]
struct ListQuery {
    status: Option<String>,
    #[serde(rename = "type")]
    workflow_type: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_limit() -> i64 {
    50
}

async fn list_workflows<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let status = q
        .status
        .as_deref()
        .and_then(|s| s.parse::<WorkflowStatus>().ok());

    let workflows = state
        .engine
        .list_workflows(status, q.workflow_type.as_deref(), q.limit, q.offset)
        .await?;

    let json: Vec<serde_json::Value> = workflows
        .into_iter()
        .map(|w| serde_json::to_value(w).unwrap_or_default())
        .collect();

    Ok(Json(json))
}

async fn describe_workflow<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let wf = state
        .engine
        .get_workflow(&id)
        .await?
        .ok_or(AppError::NotFound(format!("workflow {id}")))?;

    Ok(Json(serde_json::to_value(wf)?))
}

async fn get_events<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let events = state.engine.get_events(&id).await?;
    let json: Vec<serde_json::Value> = events
        .into_iter()
        .map(|e| serde_json::to_value(e).unwrap_or_default())
        .collect();
    Ok(Json(json))
}

#[derive(Deserialize)]
struct SignalBody {
    payload: Option<serde_json::Value>,
}

async fn send_signal<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path((id, name)): Path<(String, String)>,
    Json(body): Json<Option<SignalBody>>,
) -> Result<axum::http::StatusCode, AppError> {
    let payload = body.and_then(|b| b.payload).map(|v| v.to_string());
    state
        .engine
        .send_signal(&id, &name, payload.as_deref())
        .await?;
    Ok(axum::http::StatusCode::OK)
}

async fn cancel_workflow<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(id): Path<String>,
) -> Result<axum::http::StatusCode, AppError> {
    let cancelled = state.engine.cancel_workflow(&id).await?;
    if cancelled {
        Ok(axum::http::StatusCode::OK)
    } else {
        Err(AppError::NotFound(format!(
            "workflow {id} not found or already terminal"
        )))
    }
}

#[derive(Deserialize)]
struct TerminateBody {
    reason: Option<String>,
}

async fn terminate_workflow<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(id): Path<String>,
    Json(body): Json<Option<TerminateBody>>,
) -> Result<axum::http::StatusCode, AppError> {
    let reason = body.and_then(|b| b.reason);
    let terminated = state
        .engine
        .terminate_workflow(&id, reason.as_deref())
        .await?;
    if terminated {
        Ok(axum::http::StatusCode::OK)
    } else {
        Err(AppError::NotFound(format!(
            "workflow {id} not found or already terminal"
        )))
    }
}

// ── Error type ──────────────────────────────────────────────

pub enum AppError {
    Internal(anyhow::Error),
    NotFound(String),
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        Self::Internal(e)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::Internal(e.into())
    }
}

impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::Internal(e) => {
                tracing::error!("Internal error: {e}");
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response()
            }
            Self::NotFound(msg) => (
                axum::http::StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": format!("not found: {msg}") })),
            )
                .into_response(),
        }
    }
}
