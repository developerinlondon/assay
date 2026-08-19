//! Auth-console asset router (`/auth/...`).
//!
//! Serves the SPA shell + JS components that talk to the engine's
//! `/auth/admin/*` and `/auth/admin/oidc/*` HTTP endpoints, plus the
//! `/auth/login` browser landing for OIDC authorization-code redirects.
//!
//! Stateless on purpose — every asset is baked in via `include_str!`
//! and the index template substitution reuses the workflow dashboard's
//! whitelabel knobs.

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;

use crate::assets::{
    AUTH_API_JS, AUTH_APP_JS, AUTH_AUDIT_JS, AUTH_ICONS_SVG, AUTH_INDEX_HTML, AUTH_KEYS_JS,
    AUTH_LANDING_HTML, AUTH_LOGIN_CSS, AUTH_LOGIN_HTML, AUTH_LOGIN_JS, AUTH_OIDC_CLIENTS_JS,
    AUTH_OIDC_UPSTREAM_JS, AUTH_RECOVERY_HTML, AUTH_RECOVERY_JS, AUTH_SESSIONS_JS, AUTH_STYLE_CSS,
    AUTH_USERS_JS, AUTH_ZANZIBAR_JS, FAVICON_SVG,
};

/// Build the auth-console asset router. Stateless `Router<()>` ready
/// to merge into the engine's composed router.
///
/// All assets serve with `Cache-Control: no-cache` so a redeploy
/// invalidates client cache without manual busting (matches the
/// workflow dashboard's `router::NO_CACHE`).
pub fn router() -> Router<()> {
    console_router().merge(public_router())
}

/// Operator-only auth console assets. Public deployments can omit this
/// router while retaining browser sign-in and recovery.
pub fn console_router() -> Router<()> {
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
}

/// Browser-facing authentication assets. This excludes every operator
/// console route and is safe to mount on a public auth origin.
pub fn public_router() -> Router<()> {
    Router::new()
        .route("/auth/landing", get(landing_index))
        .route("/auth/login", get(login_index))
        .route("/auth/login/", get(login_index))
        .route("/auth/login.js", get(login_js))
        .route("/auth/login.css", get(login_css))
        .route("/auth/recovery", get(recovery_index))
        .route("/auth/recovery/", get(recovery_index))
        .route("/auth/recovery.js", get(recovery_js))
        .route("/auth/icons.svg", get(icons_svg))
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

async fn icons_svg() -> impl IntoResponse {
    asset("image/svg+xml", AUTH_ICONS_SVG)
}

async fn landing_index() -> impl IntoResponse {
    let body = crate::whitelabel::render_index(
        AUTH_LANDING_HTML,
        env!("CARGO_PKG_VERSION"),
        &crate::whitelabel::WHITELABEL,
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

async fn recovery_index() -> impl IntoResponse {
    let body = crate::whitelabel::render_index(
        AUTH_RECOVERY_HTML,
        env!("CARGO_PKG_VERSION"),
        &crate::whitelabel::WHITELABEL,
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

async fn recovery_js() -> impl IntoResponse {
    asset("application/javascript", AUTH_RECOVERY_JS)
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::assets::{
        AUTH_LANDING_HTML, AUTH_LOGIN_CSS, AUTH_LOGIN_HTML, AUTH_LOGIN_JS, AUTH_RECOVERY_HTML,
        AUTH_RECOVERY_JS,
    };

    use super::public_router;

    #[test]
    fn public_landing_exposes_only_account_entry_points() {
        assert!(AUTH_LANDING_HTML.contains("Assay Auth"));
        assert!(AUTH_LANDING_HTML.contains("href=\"/auth/login\""));
        assert!(AUTH_LANDING_HTML.contains("href=\"/auth/recovery\""));
        assert!(!AUTH_LANDING_HTML.contains("/workflow/"));
        assert!(!AUTH_LANDING_HTML.contains("admin_api_keys"));
    }

    #[tokio::test]
    async fn public_router_serves_landing_but_not_the_operator_console() {
        let landing = public_router()
            .oneshot(Request::get("/auth/landing").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(landing.status(), StatusCode::OK);
        let body = to_bytes(landing.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Secure sign-in"));

        let console = public_router()
            .oneshot(Request::get("/auth/console").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(console.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn login_page_keeps_first_party_password_auth_available() {
        assert!(AUTH_LOGIN_HTML.contains("<form id=\"password-login\""));
        assert!(AUTH_LOGIN_HTML.contains("type=\"email\""));
        assert!(AUTH_LOGIN_HTML.contains("type=\"password\""));
        assert!(AUTH_LOGIN_HTML.contains("aria-live=\"polite\""));
    }

    #[test]
    fn password_login_posts_json_with_same_origin_credentials() {
        assert!(AUTH_LOGIN_JS.contains("fetch('/api/v1/engine/auth/login'"));
        assert!(AUTH_LOGIN_JS.contains("credentials: 'same-origin'"));
        assert!(AUTH_LOGIN_JS.contains("'Content-Type': 'application/json'"));
        assert!(AUTH_LOGIN_JS.contains("email: emailInput.value"));
        assert!(AUTH_LOGIN_JS.contains("password: passwordInput.value"));
    }

    #[test]
    fn password_login_rejects_external_return_targets() {
        assert!(AUTH_LOGIN_JS.contains("function safeReturnTo"));
        assert!(AUTH_LOGIN_JS.contains("candidate.origin !== window.location.origin"));
        assert!(AUTH_LOGIN_JS.contains("return '/';"));
        assert!(AUTH_LOGIN_JS.contains("window.location.assign(returnTo)"));
    }

    #[test]
    fn login_page_links_to_password_recovery() {
        assert!(AUTH_LOGIN_HTML.contains("href=\"/auth/recovery\""));
    }

    // The story panel is a whitelabel token, never markup baked into the
    // page — an operator who configures nothing must still get a login.
    #[test]
    fn login_page_carries_the_story_as_a_token_not_as_literal_copy() {
        assert!(AUTH_LOGIN_HTML.contains("__LOGIN_STORY__"));
        assert!(AUTH_LOGIN_HTML.contains("__ACCENT_STYLE__"));
        assert!(!AUTH_LOGIN_HTML.contains("AI workforce"));
        assert!(!AUTH_LOGIN_HTML.contains("story-panel"));
    }

    // Every field keeps a real <label for=…>, and the error region stays
    // announced — the redesign must not cost the form its semantics.
    #[test]
    fn login_form_keeps_labelled_fields_and_an_announced_error_region() {
        assert!(AUTH_LOGIN_HTML.contains("<label class=\"login-label\" for=\"email\""));
        assert!(AUTH_LOGIN_HTML.contains("<label class=\"login-label\" for=\"password\""));
        assert!(AUTH_LOGIN_HTML.contains("id=\"password-error\""));
        assert!(AUTH_LOGIN_HTML.contains("role=\"alert\""));
        assert!(AUTH_LOGIN_HTML.contains("autocomplete=\"username\""));
        assert!(AUTH_LOGIN_HTML.contains("autocomplete=\"current-password\""));
    }

    // The reveal only ever flips the input's type, and a failed attempt
    // re-masks the field rather than leaving it in plain text.
    #[test]
    fn password_reveal_is_additive_and_remasks_after_a_failed_attempt() {
        assert!(AUTH_LOGIN_HTML.contains("id=\"password-reveal\""));
        assert!(AUTH_LOGIN_HTML.contains("aria-controls=\"password\""));
        assert!(AUTH_LOGIN_JS.contains("passwordInput.type = shown ? 'password' : 'text'"));
        let error_fn = AUTH_LOGIN_JS
            .find("function showPasswordError")
            .expect("error handler exists");
        let remask = AUTH_LOGIN_JS[error_fn..]
            .find("passwordInput.type = 'password'")
            .expect("failed attempt re-masks the field");
        let clears = AUTH_LOGIN_JS[error_fn..]
            .find("passwordInput.value = ''")
            .expect("failed attempt clears the field");
        assert!(clears < remask);
    }

    // login.css is the stylesheet for all three auth pages. Recovery and
    // landing put a bare .login-card in .login with no .login-panel around
    // it, so the sign-in layout must never assume that wrapper exists.
    #[test]
    fn the_shared_stylesheet_serves_pages_that_have_no_sign_in_panel() {
        for page in [AUTH_RECOVERY_HTML, AUTH_LANDING_HTML] {
            assert!(page.contains("/auth/login.css"));
            assert!(page.contains(r#"<main class="login">"#));
            assert!(!page.contains("login-panel"));
        }
        // The card carries its own measure for exactly those two pages.
        assert!(AUTH_LOGIN_CSS.contains(".login-card {"));
        let card = AUTH_LOGIN_CSS
            .find(".login-card {")
            .expect("card rule exists");
        let block_end = AUTH_LOGIN_CSS[card..].find('}').expect("rule closes");
        assert!(AUTH_LOGIN_CSS[card..card + block_end].contains("max-width"));
    }

    // A letter in a coloured square is the fallback, not the design: an
    // operator with a real mark gets it rendered instead.
    #[test]
    fn the_sign_in_badge_is_a_whitelabel_token_so_a_real_logo_can_replace_it() {
        assert!(AUTH_LOGIN_HTML.contains("__BRAND_BADGE__"));
        assert!(!AUTH_LOGIN_HTML.contains("__BRAND_MARK__"));
    }

    #[test]
    fn the_submit_button_reports_that_it_is_working_and_recovers_on_failure() {
        assert!(AUTH_LOGIN_JS.contains("passwordSubmit.disabled = true"));
        assert!(AUTH_LOGIN_JS.contains("Signing in"));
        assert!(AUTH_LOGIN_JS.contains("passwordSubmit.dataset.idleLabel"));
        assert!(AUTH_LOGIN_JS.contains("passwordSubmit.disabled = false"));
    }

    // recovery.js swaps its two forms by toggling `hidden`, and .password-login
    // declares a display, which beats the UA rule. Without this the reset page
    // shows both forms at once.
    #[test]
    fn the_hidden_attribute_survives_the_form_display_rule() {
        assert!(AUTH_LOGIN_CSS.contains("[hidden]"));
        assert!(AUTH_LOGIN_CSS.contains("display: none !important"));
        assert!(AUTH_RECOVERY_JS.contains("completeForm.hidden = false"));
    }

    // One stylesheet, one accent: a caller who follows "Forgot password?" must
    // not land on a page wearing a different brand colour.
    #[test]
    fn every_page_sharing_the_stylesheet_also_takes_the_accent() {
        for page in [AUTH_LOGIN_HTML, AUTH_RECOVERY_HTML, AUTH_LANDING_HTML] {
            assert!(page.contains("__ACCENT_STYLE__"));
        }
    }

    #[test]
    fn recovery_page_supports_request_and_completion_forms() {
        assert!(AUTH_RECOVERY_HTML.contains("<form id=\"recovery-request\""));
        assert!(AUTH_RECOVERY_HTML.contains("<form id=\"recovery-complete\""));
        assert!(AUTH_RECOVERY_HTML.contains("autocomplete=\"email\""));
        assert!(AUTH_RECOVERY_HTML.contains("autocomplete=\"new-password\""));
        assert!(AUTH_RECOVERY_HTML.contains("aria-live=\"polite\""));
    }

    #[test]
    fn recovery_controller_removes_fragment_before_using_token() {
        let read_fragment = AUTH_RECOVERY_JS
            .find("window.location.hash.slice(1)")
            .expect("controller reads fragment");
        let clear_fragment = AUTH_RECOVERY_JS
            .find("window.history.replaceState")
            .expect("controller clears fragment");
        let complete_request = AUTH_RECOVERY_JS
            .find("fetch('/api/v1/engine/auth/password/recovery/complete'")
            .expect("controller completes recovery");

        assert!(read_fragment < clear_fragment);
        assert!(clear_fragment < complete_request);
        assert!(AUTH_RECOVERY_JS.contains("fetch('/api/v1/engine/auth/password/recovery/request'"));
        assert!(AUTH_RECOVERY_JS.contains("credentials: 'same-origin'"));
    }
}
