//! HTTP routes for biscuit-share (plan 17 §S5).
//!
//! Mint + revoke are admin-key gated. Verify is intentionally NOT
//! admin-gated — share links are the public-facing surface; the
//! biscuit + revocation table are the access controls.

use axum::Router;
use axum::extract::{FromRef, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};

use assay_auth::state::AdminApiKeys;

use crate::ctx::VaultCtx;
use crate::error::VaultError;
use crate::router::{check_admin, vault_err_to_response};
use crate::share::{ShareCaveats, ShareTarget};

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    Router::new()
        .route("/share", post(mint_share::<S>))
        .route("/share/{token}", get(redeem_share::<S>))
        .route("/share/revoke", post(revoke_share::<S>))
}

#[derive(Deserialize)]
struct MintBody {
    target_kind: String,
    target_id: String,
    ttl_secs: u64,
    max_ip_cidr: Option<String>,
    max_uses: Option<u32>,
}

#[derive(Serialize)]
struct MintResponse {
    token: String,
    revocation_ids: Vec<String>,
    expires_at: f64,
}

#[derive(Serialize)]
struct RedeemResponse {
    target_kind: String,
    target_id: String,
}

#[derive(Deserialize)]
struct RevokeBody {
    revocation_id: String,
    #[serde(default)]
    reason: String,
}

async fn mint_share<S>(
    State(vault): State<VaultCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<MintBody>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    if let Err(r) = check_admin(&headers, &keys) {
        return r;
    }
    let svc = match vault.share.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("share"),
    };
    let target = match body.target_kind.as_str() {
        "item" => ShareTarget::Item(body.target_id),
        "vault" => ShareTarget::Vault(body.target_id),
        "collection" => ShareTarget::Collection(body.target_id),
        other => {
            return vault_err_to_response(VaultError::Invalid(format!(
                "unknown target_kind '{other}'; expected one of item, vault, collection"
            )));
        }
    };
    match svc.mint(
        target,
        ShareCaveats {
            ttl_secs: body.ttl_secs,
            max_ip_cidr: body.max_ip_cidr,
            max_uses: body.max_uses,
        },
    ) {
        Ok(m) => (
            StatusCode::CREATED,
            axum::Json(MintResponse {
                token: m.token,
                revocation_ids: m.revocation_ids,
                expires_at: m.expires_at,
            }),
        )
            .into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn redeem_share<S>(State(vault): State<VaultCtx>, Path(token): Path<String>) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    let svc = match vault.share.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("share"),
    };
    match svc.verify(&token, None).await {
        Ok(grant) => {
            let (kind, id) = match grant.target {
                ShareTarget::Item(id) => ("item", id),
                ShareTarget::Vault(id) => ("vault", id),
                ShareTarget::Collection(id) => ("collection", id),
            };
            axum::Json(RedeemResponse {
                target_kind: kind.to_string(),
                target_id: id,
            })
            .into_response()
        }
        Err(e) => vault_err_to_response(e),
    }
}

async fn revoke_share<S>(
    State(vault): State<VaultCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<RevokeBody>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    if let Err(r) = check_admin(&headers, &keys) {
        return r;
    }
    let svc = match vault.share.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("share"),
    };
    match svc.revoke(&body.revocation_id, &body.reason).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

fn unavailable(surface: &'static str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        axum::Json(serde_json::json!({
            "error": "service_unavailable",
            "error_description": format!("vault {surface} surface not configured"),
        })),
    )
        .into_response()
}
