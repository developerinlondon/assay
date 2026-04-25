//! Auth-console asset router (`/auth/...`).
//!
//! Serves the SPA shell + JS components that talk to the engine's
//! `/auth/admin/*` and `/auth/admin/oidc/*` HTTP endpoints. Mounted by
//! the engine binary when the auth module is active in
//! `engine.modules`.
//!
//! Stateless on purpose — every asset is baked in via `include_str!`
//! and the index template substitution reuses the workflow dashboard's
//! whitelabel knobs (so a re-skinned workflow dashboard re-skins the
//! auth console too).

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;

use crate::assets::{
    AUTH_APP_JS, AUTH_AUDIT_JS, AUTH_API_JS, AUTH_INDEX_HTML, AUTH_KEYS_JS,
    AUTH_OIDC_CLIENTS_JS, AUTH_OIDC_UPSTREAM_JS, AUTH_SESSIONS_JS, AUTH_STYLE_CSS,
    AUTH_USERS_JS, AUTH_ZANZIBAR_JS, FAVICON_SVG,
};

/// Build the auth-console asset router. Stateless — returns
/// `Router<()>` ready to merge into the engine's composed router.
///
/// Routes:
///
/// - `GET /auth/console`              → SPA shell HTML
/// - `GET /auth/console/`             → SPA shell HTML (with-trailing-slash variant)
/// - `GET /auth/style.css`            → auth-only CSS overrides
/// - `GET /auth/app.js`               → SPA controller
/// - `GET /auth/components/*.js`      → per-pane modules
///
/// All assets are served with `Cache-Control: no-cache` so a redeploy
/// invalidates client cache without manual cache-busting (matches the
/// workflow dashboard's policy — see `router::NO_CACHE`).
pub fn router() -> Router<()> {
    Router::new()
        .route("/auth/console", get(index))
        .route("/auth/console/", get(index))
        .route("/auth/style.css", get(style_css))
        .route("/auth/app.js", get(app_js))
        .route("/auth/components/api.js", get(api_js))
        .route("/auth/components/users.js", get(users_js))
        .route("/auth/components/sessions.js", get(sessions_js))
        .route("/auth/components/oidc_clients.js", get(oidc_clients_js))
        .route("/auth/components/oidc_upstream.js", get(oidc_upstream_js))
        .route("/auth/components/zanzibar.js", get(zanzibar_js))
        .route("/auth/components/keys.js", get(keys_js))
        .route("/auth/components/audit.js", get(audit_js))
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
    // Substitute the same template tokens the workflow router fills
    // when the workflow feature is on. With `auth` enabled but
    // `workflow` off (theoretical edge case), fall back to plain
    // template-token replacement here so the shell still loads. Page
    // title / footer use the unified "Assay Engine — Auth" wording so
    // operators reading the tab title can tell the three consoles
    // apart at a glance.
    #[cfg(feature = "workflow")]
    let body = {
        let asset_version = env!("CARGO_PKG_VERSION");
        crate::whitelabel::render_index(AUTH_INDEX_HTML, asset_version, &crate::whitelabel::WHITELABEL)
            .replace("Assay Workflow Dashboard", "Assay Engine — Auth")
    };
    #[cfg(not(feature = "workflow"))]
    let body = AUTH_INDEX_HTML
        .replace("__ASSETV__", env!("CARGO_PKG_VERSION"))
        .replace("__PAGE_TITLE__", "Assay Engine — Auth")
        .replace("__BRAND_NAME__", "Assay")
        .replace("__BRAND_MARK__", "A")
        .replace("__BRAND_LOGO_IMG__", "")
        .replace(
            "__FAVICON_LINK__",
            r#"<link rel="icon" type="image/svg+xml" href="/auth/favicon.svg">"#,
        )
        .replace("__EXTRA_CSS_LINK__", "")
        .replace("__DEFAULT_NAMESPACE_ATTR__", "")
        .replace(
            "__ENGINE_FOOTER__",
            r#"Powered by Assay Engine <span id="status-version">—</span>"#,
        );
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, NO_CACHE),
        ],
        body,
    )
}

async fn style_css() -> impl IntoResponse { asset("text/css", AUTH_STYLE_CSS) }
async fn app_js() -> impl IntoResponse { asset("application/javascript", AUTH_APP_JS) }
async fn api_js() -> impl IntoResponse { asset("application/javascript", AUTH_API_JS) }
async fn users_js() -> impl IntoResponse { asset("application/javascript", AUTH_USERS_JS) }
async fn sessions_js() -> impl IntoResponse { asset("application/javascript", AUTH_SESSIONS_JS) }
async fn oidc_clients_js() -> impl IntoResponse { asset("application/javascript", AUTH_OIDC_CLIENTS_JS) }
async fn oidc_upstream_js() -> impl IntoResponse { asset("application/javascript", AUTH_OIDC_UPSTREAM_JS) }
async fn zanzibar_js() -> impl IntoResponse { asset("application/javascript", AUTH_ZANZIBAR_JS) }
async fn keys_js() -> impl IntoResponse { asset("application/javascript", AUTH_KEYS_JS) }
async fn audit_js() -> impl IntoResponse { asset("application/javascript", AUTH_AUDIT_JS) }
async fn favicon() -> impl IntoResponse { asset("image/svg+xml", FAVICON_SVG) }
