//! HTTP surface for the vault module — Phase 1 ships KV + transit.
//!
//! Plan 17 §S1 (KV v2) and §S2 (transit). Mounted by the engine under
//! `/api/v1/vault/*`. Auth gating in Phase 1 is admin-key-only — every
//! route requires `Authorization: Bearer <admin-key>`. Phase 3+ adds
//! biscuit-share for delegated access and Phase 7 adds the BW-protocol
//! shim's per-user session auth.

use axum::Router;
use axum::extract::FromRef;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use assay_auth::state::AdminApiKeys;

use crate::ctx::VaultCtx;

#[cfg(feature = "vault-kv")]
mod kv;
mod sys;
#[cfg(feature = "vault-transit")]
mod transit;

/// Compose the vault HTTP router. Generic over a parent state from which
/// both [`VaultCtx`] and [`AdminApiKeys`] are extractable via `FromRef`.
/// The engine binary's `EngineState<S>` satisfies both; tests can use a
/// thin parent state — the auth crate's pattern.
pub fn vault_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    let mut r = Router::new().merge(sys::router::<S>());
    #[cfg(feature = "vault-kv")]
    {
        r = r.merge(kv::router::<S>());
    }
    #[cfg(feature = "vault-transit")]
    {
        r = r.merge(transit::router::<S>());
    }
    r
}

/// Top-of-handler admin-key check. Constant-time bytewise compare via
/// [`AdminApiKeys::check`]. Returns `Err(401)` if no `Bearer` token is
/// present or the token doesn't match.
pub(crate) fn check_admin(
    headers: &HeaderMap,
    keys: &AdminApiKeys,
) -> std::result::Result<(), Response> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    match token {
        Some(t) if keys.check(t) => Ok(()),
        _ => Err((
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({
                "error": "unauthorized",
                "error_description": "missing or invalid Bearer token",
            })),
        )
            .into_response()),
    }
}

/// Map a [`crate::error::VaultError`] to an HTTP response. Centralised
/// so KV and transit handlers stay terse.
pub(crate) fn vault_err_to_response(e: crate::error::VaultError) -> Response {
    use crate::error::VaultError as E;
    let (status, code) = match &e {
        E::NotFound => (StatusCode::NOT_FOUND, "not_found"),
        E::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
        E::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
        E::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid"),
        E::Crypto(_) => (StatusCode::INTERNAL_SERVER_ERROR, "crypto_error"),
        E::Sealed => (StatusCode::SERVICE_UNAVAILABLE, "sealed"),
        E::Backend(_) => (StatusCode::INTERNAL_SERVER_ERROR, "backend_error"),
    };
    (
        status,
        axum::Json(serde_json::json!({
            "error": code,
            "error_description": e.to_string(),
        })),
    )
        .into_response()
}
