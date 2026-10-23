use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::api::workflows::AppError;
use crate::ctx::WorkflowCtx;
use crate::store::WorkflowStore;
use crate::types::{SchedulePatch, WorkflowSchedule};

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<WorkflowCtx<S>>> {
    Router::new()
        .route("/schedules", post(create_schedule).get(list_schedules))
        .route(
            "/schedules/{name}",
            get(get_schedule)
                .patch(patch_schedule)
                .delete(delete_schedule),
        )
        .route("/schedules/{name}/pause", post(pause_schedule))
        .route("/schedules/{name}/resume", post(resume_schedule))
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
    /// Cron expression, 6 or 7 fields with seconds first
    /// (e.g. "0 0 * * * *" for hourly). The 5-field crontab form is rejected.
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
    post, path = "/api/v1/engine/workflow/schedules",
    tag = "schedules",
    request_body = CreateScheduleRequest,
    responses(
        (status = 201, description = "Schedule created", body = WorkflowSchedule),
        (status = 400, description = "Invalid cron expression or timezone"),
        (status = 500, description = "Internal error"),
    ),
)]
pub async fn create_schedule<S: WorkflowStore>(
    State(state): State<Arc<WorkflowCtx<S>>>,
    Json(req): Json<CreateScheduleRequest>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), AppError> {
    let now = timestamp_now();

    // Validate the timezone early so a bad value produces a clean 400
    // instead of a mysterious silent no-op from the scheduler later.
    if !req.timezone.eq_ignore_ascii_case("UTC") && req.timezone.parse::<chrono_tz::Tz>().is_err() {
        return Err(AppError::bad_request(format!(
            "invalid timezone: {}",
            req.timezone
        )));
    }

    // Then the cron, in that zone. A schedule that can never fire would be
    // stored with a NULL next_run_at, which the scheduler reads as due on
    // every pass — so the caller would hold a 201 for a schedule that only
    // ever logs. Refusing here is what keeps such rows out of the table.
    crate::scheduler::validate_cron_expr(&req.cron_expr, &req.timezone)
        .map_err(AppError::bad_request)?;

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

    state.create_schedule(&schedule).await?;

    // Read back so the response carries the `next_run_at` the store seeded
    // rather than the null we sent in; falls back to the request-derived
    // record if the schedule is deleted in between.
    let stored = state
        .get_schedule(&req.namespace, &req.name)
        .await?
        .unwrap_or(schedule);

    Ok((
        axum::http::StatusCode::CREATED,
        Json(serde_json::to_value(stored)?),
    ))
}

#[derive(Deserialize)]
pub struct NsQuery {
    #[serde(default = "default_namespace")]
    namespace: String,
}

#[utoipa::path(
    get, path = "/api/v1/engine/workflow/schedules",
    tag = "schedules",
    params(("namespace" = Option<String>, Query, description = "Namespace (default: main)")),
    responses((status = 200, description = "List of schedules", body = Vec<WorkflowSchedule>)),
)]
pub async fn list_schedules<S: WorkflowStore>(
    State(state): State<Arc<WorkflowCtx<S>>>,
    Query(q): Query<NsQuery>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let schedules = state.list_schedules(&q.namespace).await?;
    let json: Vec<serde_json::Value> = schedules
        .into_iter()
        .map(|s| serde_json::to_value(s).unwrap_or_default())
        .collect();
    Ok(Json(json))
}

#[utoipa::path(
    get, path = "/api/v1/engine/workflow/schedules/{name}",
    tag = "schedules",
    params(("name" = String, Path, description = "Schedule name")),
    responses(
        (status = 200, description = "Schedule details", body = WorkflowSchedule),
        (status = 404, description = "Schedule not found"),
    ),
)]
pub async fn get_schedule<S: WorkflowStore>(
    State(state): State<Arc<WorkflowCtx<S>>>,
    Path(name): Path<String>,
    Query(q): Query<NsQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let schedule = state
        .get_schedule(&q.namespace, &name)
        .await?
        .ok_or(AppError::NotFound(format!("schedule {name}")))?;

    Ok(Json(serde_json::to_value(schedule)?))
}

#[utoipa::path(
    delete, path = "/api/v1/engine/workflow/schedules/{name}",
    tag = "schedules",
    params(("name" = String, Path, description = "Schedule name")),
    responses(
        (status = 200, description = "Schedule deleted"),
        (status = 404, description = "Schedule not found"),
    ),
)]
pub async fn delete_schedule<S: WorkflowStore>(
    State(state): State<Arc<WorkflowCtx<S>>>,
    Path(name): Path<String>,
    Query(q): Query<NsQuery>,
) -> Result<axum::http::StatusCode, AppError> {
    let deleted = state.delete_schedule(&q.namespace, &name).await?;
    if deleted {
        Ok(axum::http::StatusCode::OK)
    } else {
        Err(AppError::NotFound(format!("schedule {name}")))
    }
}

#[derive(Deserialize, ToSchema)]
pub struct PatchScheduleRequest {
    /// New cron expression, 6 or 7 fields with seconds first
    /// (leave null to keep the existing one).
    pub cron_expr: Option<String>,
    /// New IANA timezone (e.g. "Europe/Berlin"; leave null to keep).
    pub timezone: Option<String>,
    /// New JSON input passed to each workflow run. Send `null` literally
    /// to preserve; use `{}` to pass an empty object.
    pub input: Option<serde_json::Value>,
    /// New task queue for created workflows.
    pub task_queue: Option<String>,
    /// New overlap policy (skip, queue, cancel_old, allow_all).
    pub overlap_policy: Option<String>,
}

#[utoipa::path(
    patch, path = "/api/v1/engine/workflow/schedules/{name}",
    tag = "schedules",
    params(
        ("name" = String, Path, description = "Schedule name"),
        ("namespace" = Option<String>, Query, description = "Namespace (default: main)"),
    ),
    request_body = PatchScheduleRequest,
    responses(
        (status = 200, description = "Schedule updated", body = WorkflowSchedule),
        (status = 400, description = "Invalid cron expression or timezone"),
        (status = 404, description = "Schedule not found"),
    ),
)]
pub async fn patch_schedule<S: WorkflowStore>(
    State(state): State<Arc<WorkflowCtx<S>>>,
    Path(name): Path<String>,
    Query(q): Query<NsQuery>,
    Json(req): Json<PatchScheduleRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Validate timezone before committing the write — same as create.
    if let Some(ref tz) = req.timezone
        && !tz.eq_ignore_ascii_case("UTC")
        && tz.parse::<chrono_tz::Tz>().is_err()
    {
        return Err(AppError::bad_request(format!("invalid timezone: {tz}")));
    }

    // The cron and the timezone are only fireable as a pair, and a patch may
    // carry either one alone, so check the effective pair against the stored
    // row: a new cron is validated in the stored zone, a new zone against the
    // stored cron. The write below still has to handle the row disappearing in
    // between.
    if req.cron_expr.is_some() || req.timezone.is_some() {
        let stored = state
            .get_schedule(&q.namespace, &name)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("schedule {name}")))?;
        let cron_expr = req.cron_expr.as_ref().unwrap_or(&stored.cron_expr);
        let timezone = req.timezone.as_ref().unwrap_or(&stored.timezone);
        crate::scheduler::validate_cron_expr(cron_expr, timezone).map_err(AppError::bad_request)?;
    }

    let patch = SchedulePatch {
        cron_expr: req.cron_expr,
        timezone: req.timezone,
        input: req.input,
        task_queue: req.task_queue,
        overlap_policy: req.overlap_policy,
    };

    let updated = state
        .update_schedule(&q.namespace, &name, &patch)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("schedule {name}")))?;

    Ok(Json(serde_json::to_value(updated)?))
}

#[utoipa::path(
    post, path = "/api/v1/engine/workflow/schedules/{name}/pause",
    tag = "schedules",
    params(
        ("name" = String, Path, description = "Schedule name"),
        ("namespace" = Option<String>, Query, description = "Namespace (default: main)"),
    ),
    responses(
        (status = 200, description = "Schedule paused", body = WorkflowSchedule),
        (status = 404, description = "Schedule not found"),
    ),
)]
pub async fn pause_schedule<S: WorkflowStore>(
    State(state): State<Arc<WorkflowCtx<S>>>,
    Path(name): Path<String>,
    Query(q): Query<NsQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let updated = state
        .set_schedule_paused(&q.namespace, &name, true)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("schedule {name}")))?;
    Ok(Json(serde_json::to_value(updated)?))
}

#[utoipa::path(
    post, path = "/api/v1/engine/workflow/schedules/{name}/resume",
    tag = "schedules",
    params(
        ("name" = String, Path, description = "Schedule name"),
        ("namespace" = Option<String>, Query, description = "Namespace (default: main)"),
    ),
    responses(
        (status = 200, description = "Schedule resumed", body = WorkflowSchedule),
        (status = 404, description = "Schedule not found"),
    ),
)]
pub async fn resume_schedule<S: WorkflowStore>(
    State(state): State<Arc<WorkflowCtx<S>>>,
    Path(name): Path<String>,
    Query(q): Query<NsQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let updated = state
        .set_schedule_paused(&q.namespace, &name, false)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("schedule {name}")))?;
    Ok(Json(serde_json::to_value(updated)?))
}

fn timestamp_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
