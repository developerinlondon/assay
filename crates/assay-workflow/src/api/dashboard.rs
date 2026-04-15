use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use std::sync::Arc;

use crate::api::AppState;
use crate::store::WorkflowStore;

const INDEX_HTML: &str = include_str!("../dashboard/index.html");
const STYLE_CSS: &str = include_str!("../dashboard/style.css");
const APP_JS: &str = include_str!("../dashboard/app.js");

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> {
    Router::new()
        .route("/workflow/", get(index))
        .route("/workflow/schedules", get(index))
        .route("/workflow/workers", get(index))
        .route("/workflow/style.css", get(style))
        .route("/workflow/app.js", get(script))
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn style() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/css")],
        STYLE_CSS,
    )
}

async fn script() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/javascript")],
        APP_JS,
    )
}
