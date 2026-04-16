use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use std::sync::Arc;
use utoipa::OpenApi;

use crate::api::AppState;
use crate::store::WorkflowStore;

/// Build the OpenAPI specification for the workflow engine.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "assay-workflow API",
        version = "0.1.0",
        description = "Durable workflow engine with REST+SSE API. Language-agnostic — any HTTP client can start workflows, execute activities, send signals.",
        license(name = "Apache-2.0"),
    ),
    paths(
        crate::api::workflows::start_workflow,
        crate::api::workflows::list_workflows,
        crate::api::workflows::describe_workflow,
        crate::api::workflows::get_events,
        crate::api::workflows::send_signal,
        crate::api::workflows::cancel_workflow,
        crate::api::workflows::terminate_workflow,
        crate::api::tasks::register_worker,
        crate::api::tasks::poll_task,
        crate::api::tasks::complete_task,
        crate::api::tasks::fail_task,
        crate::api::tasks::heartbeat_task,
        crate::api::tasks::worker_heartbeat,
        crate::api::schedules::create_schedule,
        crate::api::schedules::list_schedules,
        crate::api::schedules::get_schedule,
        crate::api::schedules::delete_schedule,
        crate::api::workflows::list_children,
        crate::api::workflows::continue_as_new,
        crate::api::workers::list_workers,
        crate::api::workers::health_check,
    ),
    components(schemas(
        crate::types::WorkflowRecord,
        crate::types::WorkflowEvent,
        crate::types::WorkflowActivity,
        crate::types::WorkflowTimer,
        crate::types::WorkflowSignal,
        crate::types::WorkflowSchedule,
        crate::types::WorkflowWorker,
        crate::types::WorkflowStatus,
        crate::types::ActivityStatus,
        crate::api::workflows::StartWorkflowRequest,
        crate::api::workflows::WorkflowResponse,
        crate::api::tasks::RegisterWorkerRequest,
        crate::api::tasks::RegisterWorkerResponse,
        crate::api::tasks::PollRequest,
        crate::api::tasks::CompleteTaskBody,
        crate::api::tasks::FailTaskBody,
        crate::api::schedules::CreateScheduleRequest,
        crate::api::workflows::ContinueAsNewBody,
    )),
    tags(
        (name = "workflows", description = "Workflow lifecycle management"),
        (name = "tasks", description = "Task execution for worker apps"),
        (name = "schedules", description = "Cron schedule management"),
        (name = "workers", description = "Worker registry and health"),
        (name = "events", description = "Real-time event streams (SSE)"),
    ),
    servers(
        (url = "/", description = "Current server"),
    ),
)]
pub struct ApiDoc;

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> {
    Router::new()
        .route("/api/v1/openapi.json", get(openapi_json))
        .route("/api/v1/docs", get(docs_page))
}

async fn openapi_json() -> impl IntoResponse {
    let spec = ApiDoc::openapi().to_json().unwrap_or_default();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        spec,
    )
}

/// Lightweight API docs page using Scalar (loaded from CDN, ~50KB).
async fn docs_page() -> Html<String> {
    let html = r#"<!DOCTYPE html>
<html>
<head>
    <title>assay-workflow API</title>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
</head>
<body>
    <script id="api-reference" data-url="/api/v1/openapi.json"></script>
    <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
</body>
</html>"#;
    Html(html.to_string())
}
