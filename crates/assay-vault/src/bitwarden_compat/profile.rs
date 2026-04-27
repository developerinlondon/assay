//! `/api/accounts/profile` and discovery endpoints (config / alive /
//! version) BW clients hit at startup.

use axum::Router;
use axum::extract::FromRef;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use assay_auth::AuthCtx;

use super::types::Profile;
use crate::ctx::VaultCtx;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AuthCtx: FromRef<S>,
{
    Router::new()
        .route("/api/accounts/profile", get(get_profile::<S>))
        .route("/api/alive", get(alive))
        .route("/api/version", get(version))
        .route("/api/config", get(config))
}

async fn alive() -> Response {
    axum::Json(serde_json::json!({
        "service": "assay-vault",
        "status": "ok",
    }))
    .into_response()
}

async fn version() -> Response {
    axum::Json(env!("CARGO_PKG_VERSION")).into_response()
}

async fn config() -> Response {
    // BW clients fetch /api/config on startup. The shape includes
    // server version, environment URLs, feature flags. Phase 7 ships
    // a minimal payload that satisfies the discovery probe.
    axum::Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "gitHash": null,
        "server": {
            "name": "assay-vault",
            "url": "/"
        },
        "environment": {
            "vault": "/",
            "api": "/api",
            "identity": "/identity",
            "notifications": "/notifications",
            "sso": ""
        },
        "featureStates": {}
    }))
    .into_response()
}

async fn get_profile<S>(
    axum::extract::State(auth): axum::extract::State<AuthCtx>,
    headers: HeaderMap,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AuthCtx: FromRef<S>,
{
    let user_id = match super::extract_user_id(&auth, &headers).await {
        Ok(uid) => uid,
        Err(r) => return r,
    };
    let user = match auth.users.get_user_by_id(&user_id).await {
        Ok(Some(u)) => u,
        _ => return super::not_found(),
    };
    let profile = Profile {
        id: user.id,
        email: user.email.clone().unwrap_or_default(),
        email_verified: user.email_verified,
        name: user.display_name,
        premium: false,
        culture: "en-US".into(),
        key: None,
        private_key: None,
        security_stamp: format!("{:x}", uuid::Uuid::new_v4().as_u128() & 0xffffffff),
        object: "profile",
    };
    axum::Json(profile).into_response()
}
