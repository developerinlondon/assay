//! HTTP surface for the vault module — Phase 1 ships KV + transit.
//!
//! Plan 17 §S1 (KV v2) and §S2 (transit). Mounted by the engine under
//! `/api/v1/vault/*`. Auth is enforced by the engine at the router
//! boundary (`vault_gate_middleware` in `assay-engine/src/server.rs`)
//! via `assay_auth::gate::require_role_for("vault", "main", "access")`,
//! which accepts the admin-key bearer (break-glass) and session +
//! zanzibar callers. Embedders consuming `vault_router` directly MUST
//! supply an equivalent gate — this router carries no per-handler
//! auth of its own. Share-redeem (`GET /share/{token}`) verifies the
//! biscuit token in the handler and is the only public path.

use axum::Router;
use axum::extract::FromRef;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::ctx::VaultCtx;

#[cfg(feature = "vault-collections")]
mod collections;
#[cfg(any(
    feature = "vault-dynamic-postgres",
    feature = "vault-dynamic-aws",
    feature = "vault-dynamic-gcp",
    feature = "vault-dynamic-kubernetes",
))]
mod dynamic;
#[cfg(feature = "vault-kv")]
mod kv;
#[cfg(feature = "vault-share")]
mod share;
mod sys;
#[cfg(feature = "vault-transit")]
mod transit;

/// Compose the vault HTTP router. Generic over a parent state from
/// which [`VaultCtx`] is extractable via `FromRef`. The engine binary's
/// `EngineState<S>` satisfies this; tests can use a thin parent state.
pub fn vault_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let mut r = Router::new().merge(sys::router::<S>());
    // BW-compat shim mounts at /identity/* + /api/* (unprefixed); the
    // engine nests THIS router under /api/v1/vault/*. The BW-compat
    // routes therefore appear under /api/v1/vault/identity/* and
    // /api/v1/vault/api/*. The engine's lib.rs additionally mounts
    // the compat router at the top level so stock BW clients (which
    // hardcode /identity and /api) can talk directly.
    #[cfg(feature = "vault-kv")]
    {
        r = r.merge(kv::router::<S>());
    }
    #[cfg(feature = "vault-transit")]
    {
        r = r.merge(transit::router::<S>());
    }
    #[cfg(feature = "vault-collections")]
    {
        r = r.merge(collections::router::<S>());
    }
    #[cfg(feature = "vault-share")]
    {
        r = r.merge(share::router::<S>());
    }
    #[cfg(any(
        feature = "vault-dynamic-postgres",
        feature = "vault-dynamic-aws",
        feature = "vault-dynamic-gcp",
        feature = "vault-dynamic-kubernetes",
    ))]
    {
        r = r.merge(dynamic::router::<S>());
    }
    r
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
