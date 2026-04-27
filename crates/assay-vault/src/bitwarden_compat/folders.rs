//! BW `/api/folders/*` — CRUD against vault.folders inside the
//! user's personal vault.

use axum::Router;
use axum::extract::{FromRef, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

use assay_auth::AuthCtx;

use super::sync::rfc3339;
use super::types::{Folder, FolderInput};
use crate::ctx::VaultCtx;
use crate::items::Parent;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AuthCtx: FromRef<S>,
{
    Router::new()
        .route("/api/folders", post(create::<S>))
        .route(
            "/api/folders/{id}",
            get(get_one::<S>).put(update::<S>).delete(delete_one::<S>),
        )
}

async fn create<S>(
    State(vault): State<VaultCtx>,
    State(auth): State<AuthCtx>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<FolderInput>,
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
    let pv = match vault.personal_vaults.as_ref() {
        Some(s) => s.clone(),
        None => return super::service_unavailable("personal_vaults"),
    };
    let folders = match vault.folders.as_ref() {
        Some(s) => s.clone(),
        None => return super::service_unavailable("folders"),
    };
    let pv_row = match pv.get_by_owner(&user_id).await {
        Ok(Some(v)) => v,
        Ok(None) => return super::not_found(),
        Err(e) => return super::vault_err(e),
    };
    let id = uuid::Uuid::now_v7().to_string();
    let folder = match folders
        .create_folder(&id, Parent::Vault(&pv_row.id), None, &input.name)
        .await
    {
        Ok(f) => f,
        Err(e) => return super::vault_err(e),
    };
    (
        StatusCode::CREATED,
        axum::Json(Folder {
            id: folder.id,
            name: folder.name,
            revision_date: rfc3339(folder.created_at),
            object: "folder",
        }),
    )
        .into_response()
}

async fn get_one<S>(
    State(vault): State<VaultCtx>,
    State(auth): State<AuthCtx>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AuthCtx: FromRef<S>,
{
    let _user_id = match super::extract_user_id(&auth, &headers).await {
        Ok(uid) => uid,
        Err(r) => return r,
    };
    let folders = match vault.folders.as_ref() {
        Some(s) => s.clone(),
        None => return super::service_unavailable("folders"),
    };
    match folders.get_folder(&id).await {
        Ok(Some(f)) => axum::Json(Folder {
            id: f.id,
            name: f.name,
            revision_date: rfc3339(f.created_at),
            object: "folder",
        })
        .into_response(),
        Ok(None) => super::not_found(),
        Err(e) => super::vault_err(e),
    }
}

async fn update<S>(
    State(vault): State<VaultCtx>,
    State(auth): State<AuthCtx>,
    headers: HeaderMap,
    Path(id): Path<String>,
    axum::Json(input): axum::Json<FolderInput>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AuthCtx: FromRef<S>,
{
    let _user_id = match super::extract_user_id(&auth, &headers).await {
        Ok(uid) => uid,
        Err(r) => return r,
    };
    let folders = match vault.folders.as_ref() {
        Some(s) => s.clone(),
        None => return super::service_unavailable("folders"),
    };
    match folders.rename_folder(&id, &input.name).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => super::not_found(),
        Err(e) => super::vault_err(e),
    }
}

async fn delete_one<S>(
    State(vault): State<VaultCtx>,
    State(auth): State<AuthCtx>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AuthCtx: FromRef<S>,
{
    let _user_id = match super::extract_user_id(&auth, &headers).await {
        Ok(uid) => uid,
        Err(r) => return r,
    };
    let folders = match vault.folders.as_ref() {
        Some(s) => s.clone(),
        None => return super::service_unavailable("folders"),
    };
    match folders.delete_folder(&id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => super::not_found(),
        Err(e) => super::vault_err(e),
    }
}
