//! Vault-console asset router (`/vault/console`).
//!
//! Plan 17 §S10. Mounted at engine root by `assay-engine` whenever the
//! `vault` feature is on AND `engine.modules.vault.enabled` is TRUE.
//!
//! Stateless: every asset is baked in via `include_str!` and the
//! index template substitution reuses the workflow dashboard's
//! whitelabel knobs.

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;

use crate::assets::{FAVICON_SVG, VAULT_APP_JS, VAULT_INDEX_HTML, VAULT_STYLE_CSS};

pub fn router() -> Router<()> {
    Router::new()
        .route("/vault/console", get(index))
        .route("/vault/console/", get(index))
        .route("/vault/console/{*path}", get(index))
        .route("/vault/style.css", get(style_css))
        .route("/vault/app.js", get(app_js))
        .route("/vault/favicon.svg", get(favicon))
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
    // Same template-substitution flow the engine console uses.
    let body = {
        let asset_version = env!("CARGO_PKG_VERSION");
        crate::whitelabel::render_index(
            VAULT_INDEX_HTML,
            asset_version,
            &crate::whitelabel::WHITELABEL,
        )
        .replace("Assay Engine — Workflow", "Assay Vault")
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

async fn style_css() -> impl IntoResponse {
    asset("text/css", VAULT_STYLE_CSS)
}

async fn app_js() -> impl IntoResponse {
    asset("application/javascript", VAULT_APP_JS)
}

async fn favicon() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, NO_CACHE),
        ],
        FAVICON_SVG,
    )
}
