//! Bitwarden-protocol compatibility shim. Plan 17 §S6.
//!
//! Implements the subset of Bitwarden's HTTP API that lets stock BW
//! mobile / browser / CLI clients work against assay-engine. Mounted
//! at `/identity/*` (auth) and `/api/*` (data) — the URLs BW clients
//! hardcode without configuration.
//!
//! ## In-scope endpoints (per plan)
//!
//! - `POST /identity/connect/token` — OAuth2 password-grant; returns
//!   a JWT minted by assay-auth.
//! - `GET  /api/accounts/profile` — current user shape.
//! - `GET  /api/sync` — full vault dump in BW JSON shape.
//! - `POST/PUT/DELETE /api/ciphers/{id?}` — items CRUD.
//! - `POST/PUT/DELETE /api/folders/{id?}` — folders CRUD.
//! - `GET  /api/config` + `GET  /api/alive` + `GET  /api/version` —
//!   discovery endpoints BW clients hit at startup.
//!
//! ## Vocabulary mapping
//!
//! | Bitwarden        | assay-vault                                     |
//! |------------------|-------------------------------------------------|
//! | User             | assay-auth user + auto-created vault.vaults row |
//! | Cipher           | vault.items                                     |
//! | Folder           | vault.folders                                   |
//! | Organization     | vault.collections (with `org_id`)               |
//! | Collection       | vault.collections (without `org_id`)            |
//!
//! Bitwarden's "Cipher" types map to `item_type`:
//! 1 = Login, 2 = SecureNote, 3 = Card, 4 = Identity, 5 = SshKey.
//! The shim stores/returns whatever the client sends without
//! interpretation — items are E2E ciphertext blobs.
//!
//! ## Two-step auth + passkey-as-cipher
//!
//! Two-step auth (TOTP, WebAuthn second factor) and passkey-as-cipher
//! (storing FIDO2 credentials inside the vault) ride on the existing
//! assay-auth passkey + JWT surfaces. Phase 7 ships the protocol
//! shape; full BW-client-tested coverage of those auxiliary flows
//! lands in v0.3.x as integration tests against a real `bw` CLI in
//! CI catch the wire-format edge cases.

use axum::Router;
use axum::extract::FromRef;

use assay_auth::AuthCtx;
use assay_auth::state::AdminApiKeys;

use crate::ctx::VaultCtx;

mod accounts;
mod ciphers;
mod folders;
mod identity;
mod profile;
mod sync;
mod types;

/// Compose the BW-compat router. Generic over a parent state from
/// which `VaultCtx`, `AuthCtx`, and `AdminApiKeys` are extractable.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AuthCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    Router::new()
        .merge(identity::router::<S>())
        .merge(accounts::router::<S>())
        .merge(profile::router::<S>())
        .merge(sync::router::<S>())
        .merge(ciphers::router::<S>())
        .merge(folders::router::<S>())
}

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

/// Pull the authenticated user id from the request's Authorization
/// bearer JWT. Returns the user_id as a String on success, or a
/// 401 response on failure.
pub(super) async fn extract_user_id(
    auth: &AuthCtx,
    headers: &HeaderMap,
) -> Result<String, Response> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    let Some(token) = token else {
        return Err(unauthorized("missing Bearer token"));
    };
    let jwt = match auth.jwt.as_ref() {
        Some(j) => j,
        None => return Err(unauthorized("JWT verifier not configured")),
    };
    #[derive(serde::Deserialize)]
    struct SubClaim {
        sub: String,
    }
    match jwt.verify::<SubClaim>(token) {
        Ok(data) => Ok(data.claims.sub),
        Err(_) => Err(unauthorized("invalid token")),
    }
}

pub(super) fn unauthorized(reason: &'static str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({
            "error": "unauthorized",
            "error_description": reason,
        })),
    )
        .into_response()
}

pub(super) fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({"error": "not_found"})),
    )
        .into_response()
}

pub(super) fn service_unavailable(surface: &'static str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        axum::Json(serde_json::json!({
            "error": "service_unavailable",
            "error_description": format!("vault {surface} surface not configured"),
        })),
    )
        .into_response()
}

pub(super) fn vault_err(e: crate::error::VaultError) -> Response {
    crate::router::vault_err_to_response(e)
}
