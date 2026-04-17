use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use utoipa::ToSchema;

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

#[derive(Deserialize, ToSchema)]
pub struct CreateScheduleRequest {
    /// Unique schedule name
    pub name: String,
    /// Namespace (default: "main")
    #[serde(default = "default_namespace")]
    pub namespace: String,
    /// Workflow type to start on each trigger
    pub workflow_type: String,
    /// Cron expression (e.g. "0 * * * *" for hourly)
    pub cron_expr: String,
    /// IANA time-zone name used to interpret `cron_expr`
    /// (e.g. "Europe/Berlin", "America/New_York"). Default: "UTC".
    #[serde(default = "default_timezone")]
    pub timezone: String,
    /// Optional JSON input passed to each workflow run
    pub input: Option<serde_json::Value>,
    /// Task queue for created workflows (default: "main")
    #[serde(default = "default_queue")]
    pub task_queue: String,
    /// Overlap policy: skip, queue, cancel_old, allow_all (default: "skip")
    #[serde(default = "default_overlap")]
    pub overlap_policy: String,
}

fn default_queue() -> String {
    "main".to_string()
}

fn default_namespace() -> String {
    "main".to_string()
}

fn default_overlap() -> String {
    "skip".to_string()
}

fn default_timezone() -> String {
    "UTC".to_string()
}

#[utoipa::path(
    post, path = "/api/v1/schedules",
    tag = "schedules",
    request_body = CreateScheduleRequest,
    responses(
        (status = 201, description = "Schedule created", body = WorkflowSchedule),
        (status = 500, description = "Internal error"),
    ),
)]
pub async fn create_schedule<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Json(req): Json<CreateScheduleRequest>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), AppError> {
    let now = timestamp_now();

    // Validate the timezone early so a bad value produces a clean 400
    // instead of a mysterious silent no-op from the scheduler later.
    if !req.timezone.eq_ignore_ascii_case("UTC")
        && req.timezone.parse::<chrono_tz::Tz>().is_err()
    {
        return Err(AppError::Internal(anyhow::anyhow!(
            "invalid timezone: {}",
            req.timezone
        )));
    }

    let schedule = WorkflowSchedule {
        name: req.name.clone(),
        namespace: req.namespace.clone(),
        workflow_type: req.workflow_type,
        cron_expr: req.cron_expr,
        timezone: req.timezone,
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

#[derive(Deserialize)]
pub struct NsQuery {
    #[serde(default = "default_namespace")]
    namespace: String,
}

#[utoipa::path(
    get, path = "/api/v1/schedules",
    tag = "schedules",
    params(("namespace" = Option<String>, Query, description = "Namespace (default: main)")),
    responses((status = 200, description = "List of schedules", body = Vec<WorkflowSchedule>)),
)]
pub async fn list_schedules<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Query(q): Query<NsQuery>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let schedules = state.engine.list_schedules(&q.namespace).await?;
    let json: Vec<serde_json::Value> = schedules
        .into_iter()
        .map(|s| serde_json::to_value(s).unwrap_or_default())
        .collect();
    Ok(Json(json))
}

#[utoipa::path(
    get, path = "/api/v1/schedules/{name}",
    tag = "schedules",
    params(("name" = String, Path, description = "Schedule name")),
    responses(
        (status = 200, description = "Schedule details", body = WorkflowSchedule),
        (status = 404, description = "Schedule not found"),
    ),
)]
pub async fn get_schedule<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(name): Path<String>,
    Query(q): Query<NsQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let schedule = state
        .engine
        .get_schedule(&q.namespace, &name)
        .await?
        .ok_or(AppError::NotFound(format!("schedule {name}")))?;

    Ok(Json(serde_json::to_value(schedule)?))
}

#[utoipa::path(
    delete, path = "/api/v1/schedules/{name}",
    tag = "schedules",
    params(("name" = String, Path, description = "Schedule name")),
    responses(
        (status = 200, description = "Schedule deleted"),
        (status = 404, description = "Schedule not found"),
    ),
)]
pub async fn delete_schedule<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(name): Path<String>,
    Query(q): Query<NsQuery>,
) -> Result<axum::http::StatusCode, AppError> {
    let deleted = state.engine.delete_schedule(&q.namespace, &name).await?;
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
