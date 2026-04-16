use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Redirect};
use axum::routing::get;
use axum::Router;
use std::sync::Arc;

use crate::api::AppState;
use crate::store::WorkflowStore;

const INDEX_HTML: &str = include_str!("../dashboard/index.html");
const THEME_CSS: &str = include_str!("../dashboard/theme.css");
const STYLE_CSS: &str = include_str!("../dashboard/style.css");
const APP_JS: &str = include_str!("../dashboard/app.js");
const WORKFLOWS_JS: &str = include_str!("../dashboard/components/workflows.js");
const DETAIL_JS: &str = include_str!("../dashboard/components/detail.js");
const SCHEDULES_JS: &str = include_str!("../dashboard/components/schedules.js");
const WORKERS_JS: &str = include_str!("../dashboard/components/workers.js");
const QUEUES_JS: &str = include_str!("../dashboard/components/queues.js");
const SETTINGS_JS: &str = include_str!("../dashboard/components/settings.js");

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> {
    Router::new()
        .route("/", get(redirect_to_dashboard))
        .route("/workflow", get(redirect_to_dashboard))
        .route("/workflow/", get(index))
        .route("/workflow/schedules", get(index))
        .route("/workflow/workers", get(index))
        .route("/workflow/queues", get(index))
        .route("/workflow/settings", get(index))
        .route("/workflow/theme.css", get(theme_css))
        .route("/workflow/style.css", get(style_css))
        .route("/workflow/app.js", get(app_js))
        .route("/workflow/components/workflows.js", get(workflows_js))
        .route("/workflow/components/detail.js", get(detail_js))
        .route("/workflow/components/schedules.js", get(schedules_js))
        .route("/workflow/components/workers.js", get(workers_js))
        .route("/workflow/components/queues.js", get(queues_js))
        .route("/workflow/components/settings.js", get(settings_js))
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn redirect_to_dashboard() -> Redirect {
    Redirect::permanent("/workflow/")
}

async fn theme_css() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/css")],
        THEME_CSS,
    )
}

async fn style_css() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/css")],
        STYLE_CSS,
    )
}

async fn app_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/javascript")],
        APP_JS,
    )
}

async fn workflows_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/javascript")],
        WORKFLOWS_JS,
    )
}

async fn detail_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/javascript")],
        DETAIL_JS,
    )
}

async fn schedules_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/javascript")],
        SCHEDULES_JS,
    )
}

async fn workers_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/javascript")],
        WORKERS_JS,
    )
}

async fn queues_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/javascript")],
        QUEUES_JS,
    )
}

async fn settings_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/javascript")],
        SETTINGS_JS,
    )
}
