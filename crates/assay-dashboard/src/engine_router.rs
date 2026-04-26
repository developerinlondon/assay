//! Engine-console asset router (`/engine/console`).
//!
//! Mounted at engine root by `assay-engine`. Always present —
//! engine-core is always running, so this router doesn't need any
//! feature gating beyond compiling against the (always-available)
//! engine asset bundle.
//!
//! Stateless: every asset is baked in via `include_str!` and the
//! index template substitution reuses the workflow dashboard's
//! whitelabel knobs (so a re-skinned workflow dashboard re-skins the
//! engine console too).

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;

use crate::assets::{
    CROSS_NAV_CSS, CROSS_NAV_JS, ENGINE_API_JS, ENGINE_APP_JS, ENGINE_AUDIT_JS,
    ENGINE_CONFIG_JS, ENGINE_INDEX_HTML, ENGINE_INFO_JS, ENGINE_INSTANCES_JS,
    ENGINE_MODULES_JS, ENGINE_STYLE_CSS, FAVICON_SVG,
};

/// Build the engine-console asset router. Stateless `Router<()>` ready
/// to merge into the engine's composed router.
///
/// `/shared/*` cross-nav assets ship from this router (the workflow +
/// auth shells just `<link>` them in) so the path stays canonical.
pub fn router() -> Router<()> {
    Router::new()
        .route("/engine/console", get(index))
        .route("/engine/console/", get(index))
        .route("/engine/console/{*path}", get(index))
        .route("/engine/style.css", get(style_css))
        .route("/engine/app.js", get(app_js))
        .route("/engine/components/api.js", get(api_js))
        .route("/engine/components/info.js", get(info_js))
        .route("/engine/components/modules.js", get(modules_js))
        .route("/engine/components/instances.js", get(instances_js))
        .route("/engine/components/audit.js", get(audit_js))
        .route("/engine/components/config.js", get(config_js))
        .route("/engine/favicon.svg", get(favicon))
        .route("/shared/cross-nav.css", get(cross_nav_css))
        .route("/shared/cross-nav.js", get(cross_nav_js))
}

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

async fn index() -> impl IntoResponse {
    // Substitute the same template tokens the workflow router fills
    // (whitelabel-aware). Title overridden to "Assay Engine — Engine"
    // so the browser tab tells operators which console they're on.
    let body = {
        let asset_version = env!("CARGO_PKG_VERSION");
        crate::whitelabel::render_index(
            ENGINE_INDEX_HTML,
            asset_version,
            &crate::whitelabel::WHITELABEL,
        )
        .replace("Assay Engine — Workflow", "Assay Engine — Engine")
    };
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, NO_CACHE),
        ],
        body,
    )
}

async fn style_css() -> impl IntoResponse { asset("text/css", ENGINE_STYLE_CSS) }
async fn app_js() -> impl IntoResponse { asset("application/javascript", ENGINE_APP_JS) }
async fn api_js() -> impl IntoResponse { asset("application/javascript", ENGINE_API_JS) }
async fn info_js() -> impl IntoResponse { asset("application/javascript", ENGINE_INFO_JS) }
async fn modules_js() -> impl IntoResponse { asset("application/javascript", ENGINE_MODULES_JS) }
async fn instances_js() -> impl IntoResponse { asset("application/javascript", ENGINE_INSTANCES_JS) }
async fn audit_js() -> impl IntoResponse { asset("application/javascript", ENGINE_AUDIT_JS) }
async fn config_js() -> impl IntoResponse { asset("application/javascript", ENGINE_CONFIG_JS) }
async fn favicon() -> impl IntoResponse { asset("image/svg+xml", FAVICON_SVG) }
async fn cross_nav_css() -> impl IntoResponse { asset("text/css", CROSS_NAV_CSS) }
async fn cross_nav_js() -> impl IntoResponse { asset("application/javascript", CROSS_NAV_JS) }
