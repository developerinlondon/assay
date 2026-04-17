use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

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
        .route("/workflows/{id}/children", get(list_children))
        .route("/workflows/{id}/continue-as-new", post(continue_as_new))
        .route("/workflows/{id}/state", get(get_workflow_state))
        .route("/workflows/{id}/state/{name}", get(get_workflow_state_by_name))
}

#[derive(Deserialize, ToSchema)]
pub struct StartWorkflowRequest {
    /// Namespace (default: "main")
    pub namespace: Option<String>,
    /// Workflow type name (e.g. "IngestData", "DeployService")
    pub workflow_type: String,
    /// Unique workflow ID (caller-provided for idempotency)
    pub workflow_id: String,
    /// Optional JSON input passed to the workflow
    pub input: Option<serde_json::Value>,
    /// Task queue to route the workflow to (default: "main")
    #[serde(default = "default_queue")]
    pub task_queue: String,
}

fn default_queue() -> String {
    "main".to_string()
}

#[derive(Serialize, ToSchema)]
pub struct WorkflowResponse {
    pub workflow_id: String,
    pub run_id: String,
    pub status: String,
}

#[utoipa::path(
    post, path = "/api/v1/workflows",
    tag = "workflows",
    request_body = StartWorkflowRequest,
    responses(
        (status = 201, description = "Workflow started", body = WorkflowResponse),
        (status = 500, description = "Internal error"),
    ),
)]
pub async fn start_workflow<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Json(req): Json<StartWorkflowRequest>,
) -> Result<(axum::http::StatusCode, Json<WorkflowResponse>), AppError> {
    let input = req.input.map(|v| v.to_string());
    let namespace = req.namespace.as_deref().unwrap_or("main");
    let wf = state
        .engine
        .start_workflow(
            namespace,
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
pub struct ListQuery {
    #[serde(default = "default_namespace")]
    pub namespace: String,
    pub status: Option<String>,
    #[serde(rename = "type")]
    pub workflow_type: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_namespace() -> String {
    "main".to_string()
}

fn default_limit() -> i64 {
    50
}

#[utoipa::path(
    get, path = "/api/v1/workflows",
    tag = "workflows",
    params(
        ("status" = Option<String>, Query, description = "Filter by status"),
        ("type" = Option<String>, Query, description = "Filter by workflow type"),
        ("limit" = Option<i64>, Query, description = "Max results (default 50)"),
        ("offset" = Option<i64>, Query, description = "Pagination offset"),
    ),
    responses(
        (status = 200, description = "List of workflows", body = Vec<WorkflowRecord>),
    ),
)]
pub async fn list_workflows<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let status = q
        .status
        .as_deref()
        .and_then(|s| s.parse::<WorkflowStatus>().ok());

    let workflows = state
        .engine
        .list_workflows(&q.namespace, status, q.workflow_type.as_deref(), q.limit, q.offset)
        .await?;

    let json: Vec<serde_json::Value> = workflows
        .into_iter()
        .map(|w| serde_json::to_value(w).unwrap_or_default())
        .collect();

    Ok(Json(json))
}

#[utoipa::path(
    get, path = "/api/v1/workflows/{id}",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "Workflow details", body = WorkflowRecord),
        (status = 404, description = "Workflow not found"),
    ),
)]
pub async fn describe_workflow<S: WorkflowStore>(
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

#[utoipa::path(
    get, path = "/api/v1/workflows/{id}/events",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "Event history", body = Vec<WorkflowEvent>),
    ),
)]
pub async fn get_events<S: WorkflowStore>(
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

#[derive(Deserialize, ToSchema)]
pub struct SignalBody {
    pub payload: Option<serde_json::Value>,
}

#[utoipa::path(
    post, path = "/api/v1/workflows/{id}/signal/{name}",
    tag = "workflows",
    params(
        ("id" = String, Path, description = "Workflow ID"),
        ("name" = String, Path, description = "Signal name"),
    ),
    responses(
        (status = 200, description = "Signal sent"),
    ),
)]
pub async fn send_signal<S: WorkflowStore>(
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

#[utoipa::path(
    post, path = "/api/v1/workflows/{id}/cancel",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "Workflow cancelled"),
        (status = 404, description = "Workflow not found or already terminal"),
    ),
)]
pub async fn cancel_workflow<S: WorkflowStore>(
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

#[derive(Deserialize, ToSchema)]
pub struct TerminateBody {
    pub reason: Option<String>,
}

#[utoipa::path(
    post, path = "/api/v1/workflows/{id}/terminate",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "Workflow terminated"),
        (status = 404, description = "Workflow not found or already terminal"),
    ),
)]
pub async fn terminate_workflow<S: WorkflowStore>(
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

#[utoipa::path(
    get, path = "/api/v1/workflows/{id}/children",
    tag = "workflows",
    params(("id" = String, Path, description = "Parent workflow ID")),
    responses(
        (status = 200, description = "Child workflows", body = Vec<WorkflowRecord>),
    ),
)]
pub async fn list_children<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let children = state.engine.list_child_workflows(&id).await?;
    let json: Vec<serde_json::Value> = children
        .into_iter()
        .map(|w| serde_json::to_value(w).unwrap_or_default())
        .collect();
    Ok(Json(json))
}

#[derive(Deserialize, ToSchema)]
pub struct ContinueAsNewBody {
    /// New input for the continued workflow run
    pub input: Option<serde_json::Value>,
}

#[utoipa::path(
    post, path = "/api/v1/workflows/{id}/continue-as-new",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID to continue")),
    request_body = ContinueAsNewBody,
    responses(
        (status = 201, description = "New workflow run started", body = WorkflowResponse),
    ),
)]
pub async fn continue_as_new<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(id): Path<String>,
    Json(body): Json<ContinueAsNewBody>,
) -> Result<(axum::http::StatusCode, Json<WorkflowResponse>), AppError> {
    let input = body.input.map(|v| v.to_string());
    let wf = state
        .engine
        .continue_as_new(&id, input.as_deref())
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

// ── Live state (register_query) ─────────────────────────────

/// Read the latest snapshot of a workflow's query-handler state.
///
/// Populated by workflow code that calls `ctx:register_query(name, fn)` —
/// each worker replay re-evaluates the registered handlers and persists the
/// combined result. Returns 404 if no workflow run has written a snapshot
/// yet (either the workflow hasn't registered any queries, or the first
/// replay hasn't completed).
#[utoipa::path(
    get, path = "/api/v1/workflows/{id}/state",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "Latest state snapshot"),
        (status = 404, description = "No snapshot recorded for this workflow"),
    ),
)]
pub async fn get_workflow_state<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let snapshot = state
        .engine
        .get_latest_snapshot(&id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("state for workflow {id}")))?;

    let parsed: serde_json::Value =
        serde_json::from_str(&snapshot.state_json).unwrap_or(serde_json::Value::Null);

    Ok(Json(serde_json::json!({
        "state": parsed,
        "event_seq": snapshot.event_seq,
        "created_at": snapshot.created_at,
    })))
}

/// Read a single named query result from a workflow's latest snapshot.
///
/// Returns the value under the given key in the latest snapshot's state
/// object, or 404 if no snapshot exists or the key is absent.
#[utoipa::path(
    get, path = "/api/v1/workflows/{id}/state/{name}",
    tag = "workflows",
    params(
        ("id" = String, Path, description = "Workflow ID"),
        ("name" = String, Path, description = "Query handler name"),
    ),
    responses(
        (status = 200, description = "Query value"),
        (status = 404, description = "No snapshot or key not present"),
    ),
)]
pub async fn get_workflow_state_by_name<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path((id, name)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let snapshot = state
        .engine
        .get_latest_snapshot(&id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("state for workflow {id}")))?;

    let parsed: serde_json::Value =
        serde_json::from_str(&snapshot.state_json).unwrap_or(serde_json::Value::Null);

    let value = parsed
        .get(&name)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("query '{name}' for workflow {id}")))?;

    Ok(Json(serde_json::json!({
        "value": value,
        "event_seq": snapshot.event_seq,
        "created_at": snapshot.created_at,
    })))
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

// Type alias for utoipa references (the actual type is WorkflowRecord from types.rs)
use crate::types::{WorkflowEvent, WorkflowRecord};
