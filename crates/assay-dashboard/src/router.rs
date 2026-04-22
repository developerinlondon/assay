use std::sync::Arc;

use axum::Router;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Redirect};
use axum::routing::get;

use crate::assets::{
    ACTIONS_JS, APP_JS, DETAIL_JS, FAVICON_SVG, MODAL_JS, QUEUES_JS, SCHEDULES_JS, SELECT_JS,
    SETTINGS_JS, STYLE_CSS, THEME_CSS, WORKERS_JS, WORKFLOWS_JS,
};
use crate::ctx::DashboardCtx;
use crate::whitelabel::{render_index, WHITELABEL};
use crate::assets::INDEX_HTML;

/// Build the axum router that serves the workflow dashboard assets.
///
/// State type is `Arc<DashboardCtx>`. Wire this into a parent router
/// with `.merge(assay_dashboard::workflow_router().with_state(ctx))`.
pub fn router() -> Router<Arc<DashboardCtx>> {
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
        .route("/workflow/components/modal.js", get(modal_js))
        .route("/workflow/components/actions.js", get(actions_js))
        .route("/workflow/components/select.js", get(select_js))
        .route("/workflow/favicon.svg", get(favicon))
        .route("/favicon.ico", get(favicon))
}

/// Cache-control for dashboard assets: tell CDNs/browsers to revalidate every
/// request. The dashboard is small and embedded in the binary, so the cost of
/// re-checking is trivial — and it prevents stale UI after a redeploy.
const NO_CACHE: &str = "no-cache, no-store, must-revalidate";

fn asset(content_type: &'static str, body: &'static str) -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, NO_CACHE),
        ],
        body,
    )
}

async fn index(axum::extract::State(ctx): axum::extract::State<Arc<DashboardCtx>>) -> impl IntoResponse {
    let body = render_index(INDEX_HTML, ctx.asset_version.as_str(), &WHITELABEL);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, NO_CACHE),
        ],
        body,
    )
}

async fn redirect_to_dashboard() -> Redirect {
    Redirect::permanent("/workflow/")
}

async fn theme_css() -> impl IntoResponse { asset("text/css", THEME_CSS) }
async fn style_css() -> impl IntoResponse { asset("text/css", STYLE_CSS) }
async fn app_js() -> impl IntoResponse { asset("application/javascript", APP_JS) }
async fn workflows_js() -> impl IntoResponse { asset("application/javascript", WORKFLOWS_JS) }
async fn detail_js() -> impl IntoResponse { asset("application/javascript", DETAIL_JS) }
async fn schedules_js() -> impl IntoResponse { asset("application/javascript", SCHEDULES_JS) }
async fn workers_js() -> impl IntoResponse { asset("application/javascript", WORKERS_JS) }
async fn queues_js() -> impl IntoResponse { asset("application/javascript", QUEUES_JS) }
async fn settings_js() -> impl IntoResponse { asset("application/javascript", SETTINGS_JS) }
async fn modal_js() -> impl IntoResponse { asset("application/javascript", MODAL_JS) }
async fn actions_js() -> impl IntoResponse { asset("application/javascript", ACTIONS_JS) }
async fn select_js() -> impl IntoResponse { asset("application/javascript", SELECT_JS) }
async fn favicon() -> impl IntoResponse { asset("image/svg+xml", FAVICON_SVG) }
