//! KV v2 HTTP handlers (plan 17 §S1).
//!
//! Wire shape:
//!
//! ```text
//! PUT    /api/v1/vault/kv/*path                    body: { data, custom_md? }
//! GET    /api/v1/vault/kv/*path?version=N
//! GET    /api/v1/vault/kv-list/*prefix             list under a prefix
//! DELETE /api/v1/vault/kv/*path?version=N          soft-delete
//! POST   /api/v1/vault/kv/*path/destroy?version=N  hard-destroy
//! POST   /api/v1/vault/kv/*path/undelete?version=N
//! ```
//!
//! The plan locks the wire surface to `/api/v1/vault/kv/{path}` for
//! both PUT and GET, and `/api/v1/vault/kv/{prefix}` for LIST. axum's
//! catch-all `*path` placeholder collides with literal sub-paths
//! (`/destroy`, `/undelete`), so we route those through `:` separators
//! handled below — or a sibling `/kv-list/` prefix for LIST so the
//! routing tree stays unambiguous. Everything else uses the
//! plan-locked shape.

use axum::Router;
use axum::extract::{FromRef, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use assay_auth::state::AdminApiKeys;

use crate::ctx::{DynKvStore, VaultCtx};
use crate::error::VaultError;
use crate::router::{check_admin, vault_err_to_response};

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    Router::new()
        .route(
            "/kv/{*path}",
            put(put_kv::<S>).get(get_kv::<S>).delete(delete_kv::<S>),
        )
        .route("/kv-list/{*prefix}", get(list_kv::<S>))
        .route("/kv-list", get(list_kv_root::<S>))
        .route("/kv-meta/{*path}", get(meta_kv::<S>))
        .route("/kv-destroy/{*path}", post(destroy_kv::<S>))
        .route("/kv-undelete/{*path}", post(undelete_kv::<S>))
}

#[derive(Deserialize)]
struct PutBody {
    /// Plaintext payload as a UTF-8 string. Callers who need binary
    /// safety encode their own outer base64 — Phase 1 keeps the wire
    /// untyped so JSON-friendly secrets round-trip without escaping.
    data: String,
    #[serde(default = "empty_obj")]
    custom_md: Value,
}

fn empty_obj() -> Value {
    Value::Object(Default::default())
}

#[derive(Serialize)]
struct PutResponse {
    path: String,
    version: i64,
}

#[derive(Serialize)]
struct GetResponse {
    path: String,
    version: i64,
    data: String,
    deleted_at: Option<f64>,
    created_at: f64,
}

#[derive(Deserialize)]
struct VersionQuery {
    version: Option<i64>,
}

async fn put_kv<S>(
    State(vault): State<VaultCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(path): Path<String>,
    axum::Json(body): axum::Json<PutBody>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    if let Err(r) = check_admin(&headers, &keys) {
        return r;
    }
    let kv = match vault.kv.as_ref() {
        Some(k) => k,
        None => return service_unavailable("kv"),
    };
    match kv.put(&path, body.data.as_bytes(), body.custom_md).await {
        Ok(version) => (
            StatusCode::CREATED,
            axum::Json(PutResponse { path, version }),
        )
            .into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn get_kv<S>(
    State(vault): State<VaultCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Query(q): Query<VersionQuery>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    if let Err(r) = check_admin(&headers, &keys) {
        return r;
    }
    let kv = match vault.kv.as_ref() {
        Some(k) => k,
        None => return service_unavailable("kv"),
    };
    match kv.get(&path, q.version).await {
        Ok(read) => {
            let data = match String::from_utf8(read.plaintext) {
                Ok(s) => s,
                Err(e) => {
                    return vault_err_to_response(VaultError::Invalid(format!(
                        "stored payload is not valid UTF-8: {e}"
                    )));
                }
            };
            axum::Json(GetResponse {
                path: read.path,
                version: read.version,
                data,
                deleted_at: read.deleted_at,
                created_at: read.created_at,
            })
            .into_response()
        }
        Err(e) => vault_err_to_response(e),
    }
}

async fn delete_kv<S>(
    State(vault): State<VaultCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Query(q): Query<VersionQuery>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    if let Err(r) = check_admin(&headers, &keys) {
        return r;
    }
    let kv = match vault.kv.as_ref() {
        Some(k) => k,
        None => return service_unavailable("kv"),
    };
    let version = match q.version {
        Some(v) => v,
        None => {
            return vault_err_to_response(VaultError::Invalid(
                "?version=N is required for soft-delete".into(),
            ));
        }
    };
    match kv.soft_delete(&path, version).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn destroy_kv<S>(
    State(vault): State<VaultCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Query(q): Query<VersionQuery>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    if let Err(r) = check_admin(&headers, &keys) {
        return r;
    }
    let kv = match vault.kv.as_ref() {
        Some(k) => k,
        None => return service_unavailable("kv"),
    };
    let version = match q.version {
        Some(v) => v,
        None => {
            return vault_err_to_response(VaultError::Invalid(
                "?version=N is required for destroy".into(),
            ));
        }
    };
    match kv.destroy(&path, version).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn undelete_kv<S>(
    State(vault): State<VaultCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Query(q): Query<VersionQuery>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    if let Err(r) = check_admin(&headers, &keys) {
        return r;
    }
    let kv = match vault.kv.as_ref() {
        Some(k) => k,
        None => return service_unavailable("kv"),
    };
    let version = match q.version {
        Some(v) => v,
        None => {
            return vault_err_to_response(VaultError::Invalid(
                "?version=N is required for undelete".into(),
            ));
        }
    };
    match kv.undelete(&path, version).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn list_kv<S>(
    State(vault): State<VaultCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(prefix): Path<String>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    if let Err(r) = check_admin(&headers, &keys) {
        return r;
    }
    list_impl(&vault, &prefix).await
}

async fn list_kv_root<S>(
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
    list_impl(&vault, "").await
}

async fn list_impl(vault: &VaultCtx, prefix: &str) -> Response {
    let kv = match vault.kv.as_ref() {
        Some(k) => k,
        None => return service_unavailable("kv"),
    };
    match kv.list(prefix).await {
        Ok(entries) => axum::Json(serde_json::json!({ "entries": entries })).into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn meta_kv<S>(
    State(vault): State<VaultCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    if let Err(r) = check_admin(&headers, &keys) {
        return r;
    }
    let kv = match vault.kv.as_ref() {
        Some(k) => k,
        None => return service_unavailable("kv"),
    };
    match kv.read_meta(&path).await {
        Ok(m) => axum::Json(m).into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

fn service_unavailable(surface: &'static str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        axum::Json(serde_json::json!({
            "error": "service_unavailable",
            "error_description": format!("vault {surface} surface not configured"),
        })),
    )
        .into_response()
}

// Suppress unused warnings on DynKvStore re-export — referenced
// transitively by ctx types, useful for handlers in later phases.
#[allow(dead_code)]
type _Phase1KvCheck = DynKvStore;
