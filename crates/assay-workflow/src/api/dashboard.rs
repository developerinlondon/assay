use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Redirect};
use axum::routing::get;
use axum::Router;
use std::sync::{Arc, LazyLock};
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Inline SVG favicon — single accent-coloured "A" mark on a dark surface.
/// Browsers fetch this for the tab icon and (in collapsed mode) it doubles as
/// our visual identity.
const FAVICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="12" fill="#0d1117"/><text x="32" y="46" font-family="-apple-system,BlinkMacSystemFont,Segoe UI,Helvetica,Arial,sans-serif" font-size="44" font-weight="800" fill="#e6662a" text-anchor="middle">A</text></svg>"##;

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
        .route("/workflow/favicon.svg", get(favicon))
        .route("/favicon.ico", get(favicon))
}

/// Cache-control for dashboard assets: tell CDNs/browsers to revalidate every
/// request. The dashboard is small and embedded in the binary, so the cost of
/// re-checking is trivial — and it prevents stale UI after a redeploy.
const NO_CACHE: &str = "no-cache, no-store, must-revalidate";

/// Per-process asset version stamp. Embedded into the served HTML so that every
/// engine restart produces unique asset URLs (e.g. `style.css?v=1776338400`).
/// This breaks both browser and CDN caches automatically — without it, an
/// upstream proxy that ignored Cache-Control would keep serving stale JS/CSS
/// after a redeploy.
static ASSET_VERSION: LazyLock<String> = LazyLock::new(|| {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string())
});

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

async fn index() -> impl IntoResponse {
    let body = INDEX_HTML.replace("__ASSETV__", ASSET_VERSION.as_str());
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
async fn favicon() -> impl IntoResponse { asset("image/svg+xml", FAVICON_SVG) }
