//! `/auth/login` asset router.
//!
//! Per the decoupled-modules architecture, the engine no longer hosts
//! operator SPAs (sysops/gondor does). The only browser-facing asset
//! that stays on the engine is the OIDC login page — it's the target
//! of the `/authorize` redirect, so it has to live where the OIDC
//! authorization-code flow can find it.
//!
//! Other handlers in this file (index, app_js, component bundles) are
//! retained for the moment but no longer mounted; Stage 5 of the
//! decoupling refactor removes the whole assay-dashboard crate.
#![allow(dead_code)]

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;

use crate::assets::{
    AUTH_API_JS, AUTH_APP_JS, AUTH_AUDIT_JS, AUTH_INDEX_HTML, AUTH_KEYS_JS, AUTH_LOGIN_CSS,
    AUTH_LOGIN_HTML, AUTH_LOGIN_JS, AUTH_OIDC_CLIENTS_JS, AUTH_OIDC_UPSTREAM_JS, AUTH_SESSIONS_JS,
    AUTH_STYLE_CSS, AUTH_USERS_JS, AUTH_ZANZIBAR_JS, FAVICON_SVG,
};

/// Build the auth-console asset router. Stateless `Router<()>` ready
/// to merge into the engine's composed router.
///
/// All assets serve with `Cache-Control: no-cache` so a redeploy
/// invalidates client cache without manual busting (matches the
/// workflow dashboard's `router::NO_CACHE`).
pub fn router() -> Router<()> {
    Router::new()
        // Login landing — target of assay_auth's `return_to_for(...)`
        // redirect for unauthenticated /authorize. Browser-facing by
        // design.
        .route("/auth/login", get(login_index))
        .route("/auth/login/", get(login_index))
        .route("/auth/login.js", get(login_js))
        .route("/auth/login.css", get(login_css))
        .route("/auth/favicon.svg", get(favicon))
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
    // Substitute the same template tokens the workflow router fills.
    // Page title / footer use the unified "Assay Engine — Auth"
    // wording so operators reading the tab title can tell the three
    // consoles apart at a glance.
    let body = {
        let asset_version = env!("CARGO_PKG_VERSION");
        crate::whitelabel::render_index(
            AUTH_INDEX_HTML,
            asset_version,
            &crate::whitelabel::WHITELABEL,
        )
        .replace("Assay Workflow Dashboard", "Assay Engine — Auth")
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
    asset("text/css", AUTH_STYLE_CSS)
}
async fn app_js() -> impl IntoResponse {
    asset("application/javascript", AUTH_APP_JS)
}
async fn api_js() -> impl IntoResponse {
    asset("application/javascript", AUTH_API_JS)
}
async fn users_js() -> impl IntoResponse {
    asset("application/javascript", AUTH_USERS_JS)
}
async fn sessions_js() -> impl IntoResponse {
    asset("application/javascript", AUTH_SESSIONS_JS)
}
async fn oidc_clients_js() -> impl IntoResponse {
    asset("application/javascript", AUTH_OIDC_CLIENTS_JS)
}
async fn oidc_upstream_js() -> impl IntoResponse {
    asset("application/javascript", AUTH_OIDC_UPSTREAM_JS)
}
async fn zanzibar_js() -> impl IntoResponse {
    asset("application/javascript", AUTH_ZANZIBAR_JS)
}
async fn keys_js() -> impl IntoResponse {
    asset("application/javascript", AUTH_KEYS_JS)
}
async fn audit_js() -> impl IntoResponse {
    asset("application/javascript", AUTH_AUDIT_JS)
}
async fn favicon() -> impl IntoResponse {
    asset("image/svg+xml", FAVICON_SVG)
}

async fn login_index() -> impl IntoResponse {
    // The login template carries its own literal title token
    // (`Sign in · __BRAND_NAME__`), so we don't need the brittle
    // post-render `.replace(...)` the admin index uses.
    let body = {
        let asset_version = env!("CARGO_PKG_VERSION");
        crate::whitelabel::render_index(
            AUTH_LOGIN_HTML,
            asset_version,
            &crate::whitelabel::WHITELABEL,
        )
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

async fn login_js() -> impl IntoResponse {
    asset("application/javascript", AUTH_LOGIN_JS)
}

async fn login_css() -> impl IntoResponse {
    asset("text/css", AUTH_LOGIN_CSS)
}
