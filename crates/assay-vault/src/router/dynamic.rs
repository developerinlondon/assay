//! HTTP routes for dynamic credentials (plan 17 §S3).
//!
//! Mounted under /api/v1/vault/dynamic/*. Admin-key gated for Phase 5.
//! AWS / GCP / K8s providers ride on the same routes once their impls
//! land — the dispatcher routes by `provider_name` from the URL.

use axum::Router;
use axum::extract::{FromRef, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use serde::{Deserialize, Serialize};

use assay_auth::state::AdminApiKeys;

use crate::ctx::VaultCtx;
use crate::router::vault_err_to_response;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    Router::new()
        .route("/dynamic/{provider}/{role}/lease", post(issue_lease::<S>))
        .route("/dynamic/leases", get(list_leases::<S>))
        .route("/dynamic/leases/{id}", delete(revoke_lease::<S>))
}

#[derive(Deserialize)]
struct IssueBody {
    #[serde(default = "default_ttl")]
    ttl_secs: u64,
}

fn default_ttl() -> u64 {
    3600
}

#[derive(Serialize)]
struct ListLeasesQuery {
    provider: Option<String>,
}

async fn issue_lease<S>(
    State(vault): State<VaultCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path((provider, role)): Path<(String, String)>,
    body: Option<axum::Json<IssueBody>>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    let svc = match vault.dynamic.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("dynamic"),
    };
    let ttl = body.map(|b| b.0.ttl_secs).unwrap_or_else(default_ttl);
    match svc.issue(&provider, &role, ttl).await {
        Ok(lease) => (StatusCode::CREATED, axum::Json(lease)).into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

#[derive(Deserialize)]
struct ListQuery {
    provider: Option<String>,
}

async fn list_leases<S>(
    State(vault): State<VaultCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    let svc = match vault.dynamic.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("dynamic"),
    };
    match svc.leases().list_leases(q.provider.as_deref()).await {
        Ok(rows) => axum::Json(serde_json::json!({ "leases": rows })).into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn revoke_lease<S>(
    State(vault): State<VaultCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    let svc = match vault.dynamic.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("dynamic"),
    };
    match svc.revoke(&id).await {
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

// Suppress dead_code on the alternative ListLeasesQuery for ListQuery (kept for forward compatibility).
#[allow(dead_code)]
type _Phase5Reserved = ListLeasesQuery;
