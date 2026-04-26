//! Sys-routes for sealing — seal-status / init / unseal / seal.
//!
//! Plan 17 §S7. Mounted under `/api/v1/vault/sys/*`. Admin-key gated
//! for Phase 2; the init / seal / unseal endpoints are operator-level
//! actions that bypass the per-request seal gate (you can't unseal a
//! sealed vault if every endpoint refuses sealed access).

use axum::Router;
use axum::extract::{FromRef, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};

use assay_auth::state::AdminApiKeys;

use crate::ctx::VaultCtx;
use crate::error::VaultError;
use crate::router::{check_admin, vault_err_to_response};

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    Router::new()
        .route("/sys/seal-status", get(seal_status::<S>))
        .route("/sys/seal", post(seal_op::<S>))
        .route("/sys/unseal", post(unseal_op::<S>))
}

#[derive(Serialize)]
struct SealStatusResponse {
    sealed: bool,
    method: String,
    kid: Option<String>,
    shares_progress: u8,
    share_threshold: Option<u8>,
    share_count: Option<u8>,
}

async fn seal_status<S>(
    State(vault): State<VaultCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    if let Err(r) = check_admin(&headers, &keys) {
        return r;
    }
    let st = vault.seal_state.status();
    axum::Json(SealStatusResponse {
        sealed: st.sealed,
        method: st.method.as_column().to_string(),
        kid: st.kid,
        shares_progress: st.shares_progress,
        share_threshold: st.share_threshold,
        share_count: st.share_count,
    })
    .into_response()
}

async fn seal_op<S>(
    State(vault): State<VaultCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    if let Err(r) = check_admin(&headers, &keys) {
        return r;
    }
    if let Err(e) = vault.seal_state.seal() {
        return vault_err_to_response(e);
    }
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
struct UnsealBody {
    /// Base64-encoded share bytes — exactly the shape returned by an
    /// init ceremony (each share is one entry from the
    /// `crypto::sealing::shamir::Share` collection).
    share_b64: String,
}

async fn unseal_op<S>(
    State(vault): State<VaultCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<UnsealBody>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    if let Err(r) = check_admin(&headers, &keys) {
        return r;
    }
    #[cfg(feature = "vault-sealing-shamir")]
    {
        let share_bytes = match data_encoding::BASE64.decode(body.share_b64.as_bytes()) {
            Ok(b) => b,
            Err(_) => {
                return vault_err_to_response(VaultError::Invalid(
                    "share_b64 is not valid base64".into(),
                ));
            }
        };
        match vault.seal_state.submit_shamir_share(share_bytes) {
            Ok(st) => axum::Json(SealStatusResponse {
                sealed: st.sealed,
                method: st.method.as_column().to_string(),
                kid: st.kid,
                shares_progress: st.shares_progress,
                share_threshold: st.share_threshold,
                share_count: st.share_count,
            })
            .into_response(),
            Err(e) => vault_err_to_response(e),
        }
    }
    #[cfg(not(feature = "vault-sealing-shamir"))]
    {
        let _ = body;
        vault_err_to_response(VaultError::Invalid(
            "vault-sealing-shamir feature is not enabled in this build".into(),
        ))
    }
}
