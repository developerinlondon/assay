//! HTTP routes for personal vault + collections + items + folders
//! (plan 17 §S4). Admin-key gated for Phase 3; Phase 7 BW-compat
//! adds session auth so individual users can manage their own.

use axum::Router;
use axum::extract::{FromRef, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use serde::Deserialize;

use crate::ctx::VaultCtx;
use crate::error::VaultError;
use crate::items::Parent;
use crate::router::vault_err_to_response;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    Router::new()
        // ── Personal vault ────────────────────────────────────
        .route(
            "/me/{user_id}",
            post(ensure_personal_vault::<S>).get(get_personal_vault::<S>),
        )
        .route(
            "/me/{user_id}/items",
            post(create_personal_item::<S>).get(list_personal_items::<S>),
        )
        // ── Collections ──────────────────────────────────────
        .route(
            "/collections",
            post(create_collection::<S>).get(list_collections::<S>),
        )
        .route(
            "/collections/{id}",
            get(get_collection::<S>).delete(delete_collection::<S>),
        )
        .route(
            "/collections/{id}/members",
            post(upsert_member::<S>).get(list_members::<S>),
        )
        .route(
            "/collections/{id}/members/{user_id}",
            delete(remove_member::<S>),
        )
        .route(
            "/collections/{id}/items",
            post(create_collection_item::<S>).get(list_collection_items::<S>),
        )
        // ── Items + folders (id-keyed) ───────────────────────
        .route(
            "/items/{id}",
            get(get_item::<S>)
                .put(update_item::<S>)
                .delete(delete_item::<S>),
        )
        .route("/folders", post(create_folder::<S>))
        .route(
            "/folders/{id}",
            get(get_folder::<S>)
                .put(rename_folder::<S>)
                .delete(delete_folder::<S>),
        )
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

fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

// ─────────────────────────────────────────────────────────────
// Personal vault
// ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct EnsureVaultBody {
    public_key_b64: String,
}

async fn ensure_personal_vault<S>(
    State(vault): State<VaultCtx>,
    Path(user_id): Path<String>,
    axum::Json(body): axum::Json<EnsureVaultBody>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let store = match vault.personal_vaults.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("personal_vaults"),
    };
    let pubkey = match data_encoding::BASE64.decode(body.public_key_b64.as_bytes()) {
        Ok(b) => b,
        Err(_) => {
            return vault_err_to_response(VaultError::Invalid(
                "public_key_b64 is not valid base64".into(),
            ));
        }
    };
    let id = new_id();
    match store.ensure_vault(&id, &user_id, &pubkey).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn get_personal_vault<S>(
    State(vault): State<VaultCtx>,
    Path(user_id): Path<String>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let store = match vault.personal_vaults.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("personal_vaults"),
    };
    match store.get_by_owner(&user_id).await {
        Ok(Some(v)) => axum::Json(v).into_response(),
        Ok(None) => vault_err_to_response(VaultError::NotFound),
        Err(e) => vault_err_to_response(e),
    }
}

#[derive(Deserialize)]
struct CreateItemBody {
    item_type: String,
    name: String,
    /// Pre-encrypted ciphertext, base64.
    ciphertext_b64: String,
    /// AEAD nonce, base64. The client owns the AEAD scheme; the
    /// server is just a blob store for items.
    nonce_b64: String,
    folder_id: Option<String>,
}

async fn create_personal_item<S>(
    State(vault): State<VaultCtx>,
    Path(user_id): Path<String>,
    axum::Json(body): axum::Json<CreateItemBody>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let pv = match vault.personal_vaults.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("personal_vaults"),
    };
    let items = match vault.items.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("items"),
    };
    let v = match pv.get_by_owner(&user_id).await {
        Ok(Some(v)) => v,
        Ok(None) => {
            return vault_err_to_response(VaultError::NotFound);
        }
        Err(e) => return vault_err_to_response(e),
    };
    let (ct, nonce) = match decode_pair(&body.ciphertext_b64, &body.nonce_b64) {
        Ok(p) => p,
        Err(e) => return vault_err_to_response(e),
    };
    let id = new_id();
    match items
        .create_item(
            &id,
            Parent::Vault(&v.id),
            body.folder_id.as_deref(),
            &body.item_type,
            &body.name,
            &ct,
            &nonce,
        )
        .await
    {
        Ok(item) => (StatusCode::CREATED, axum::Json(item)).into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn list_personal_items<S>(
    State(vault): State<VaultCtx>,
    Path(user_id): Path<String>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let pv = match vault.personal_vaults.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("personal_vaults"),
    };
    let items = match vault.items.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("items"),
    };
    let v = match pv.get_by_owner(&user_id).await {
        Ok(Some(v)) => v,
        Ok(None) => return vault_err_to_response(VaultError::NotFound),
        Err(e) => return vault_err_to_response(e),
    };
    match items.list_items(Parent::Vault(&v.id)).await {
        Ok(rows) => axum::Json(serde_json::json!({ "items": rows })).into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

// ─────────────────────────────────────────────────────────────
// Collections
// ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateCollectionBody {
    org_id: Option<String>,
    name: String,
    created_by: String,
}

async fn create_collection<S>(
    State(vault): State<VaultCtx>,
    axum::Json(body): axum::Json<CreateCollectionBody>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let store = match vault.collections.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("collections"),
    };
    let id = new_id();
    match store
        .create_collection(&id, body.org_id.as_deref(), &body.name, &body.created_by)
        .await
    {
        Ok(c) => (StatusCode::CREATED, axum::Json(c)).into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

#[derive(Deserialize)]
struct ListCollectionsQuery {
    org_id: Option<String>,
}

async fn list_collections<S>(
    State(vault): State<VaultCtx>,
    Query(q): Query<ListCollectionsQuery>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let store = match vault.collections.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("collections"),
    };
    match store.list_collections(q.org_id.as_deref()).await {
        Ok(rows) => axum::Json(serde_json::json!({ "collections": rows })).into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn get_collection<S>(State(vault): State<VaultCtx>, Path(id): Path<String>) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let store = match vault.collections.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("collections"),
    };
    match store.get_collection(&id).await {
        Ok(Some(c)) => axum::Json(c).into_response(),
        Ok(None) => vault_err_to_response(VaultError::NotFound),
        Err(e) => vault_err_to_response(e),
    }
}

async fn delete_collection<S>(State(vault): State<VaultCtx>, Path(id): Path<String>) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let store = match vault.collections.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("collections"),
    };
    match store.delete_collection(&id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => vault_err_to_response(VaultError::NotFound),
        Err(e) => vault_err_to_response(e),
    }
}

#[derive(Deserialize)]
struct UpsertMemberBody {
    user_id: String,
    /// Wrapped collection key, base64 — opaque to the server.
    wrapped_key_b64: String,
    role: Option<String>,
}

async fn upsert_member<S>(
    State(vault): State<VaultCtx>,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<UpsertMemberBody>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let store = match vault.collections.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("collections"),
    };
    let wrapped = match data_encoding::BASE64.decode(body.wrapped_key_b64.as_bytes()) {
        Ok(b) => b,
        Err(_) => {
            return vault_err_to_response(VaultError::Invalid(
                "wrapped_key_b64 is not valid base64".into(),
            ));
        }
    };
    let role = body
        .role
        .as_deref()
        .unwrap_or(crate::collections::roles::VIEWER);
    match store
        .upsert_member(&id, &body.user_id, &wrapped, role)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn list_members<S>(State(vault): State<VaultCtx>, Path(id): Path<String>) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let store = match vault.collections.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("collections"),
    };
    match store.list_members(&id).await {
        Ok(members) => axum::Json(serde_json::json!({ "members": members })).into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn remove_member<S>(
    State(vault): State<VaultCtx>,
    Path((id, user_id)): Path<(String, String)>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let store = match vault.collections.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("collections"),
    };
    match store.remove_member(&id, &user_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => vault_err_to_response(VaultError::NotFound),
        Err(e) => vault_err_to_response(e),
    }
}

// ─────────────────────────────────────────────────────────────
// Collection items
// ─────────────────────────────────────────────────────────────

async fn create_collection_item<S>(
    State(vault): State<VaultCtx>,
    Path(collection_id): Path<String>,
    axum::Json(body): axum::Json<CreateItemBody>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let items = match vault.items.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("items"),
    };
    let (ct, nonce) = match decode_pair(&body.ciphertext_b64, &body.nonce_b64) {
        Ok(p) => p,
        Err(e) => return vault_err_to_response(e),
    };
    let id = new_id();
    match items
        .create_item(
            &id,
            Parent::Collection(&collection_id),
            body.folder_id.as_deref(),
            &body.item_type,
            &body.name,
            &ct,
            &nonce,
        )
        .await
    {
        Ok(item) => (StatusCode::CREATED, axum::Json(item)).into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn list_collection_items<S>(
    State(vault): State<VaultCtx>,
    Path(collection_id): Path<String>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let items = match vault.items.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("items"),
    };
    match items.list_items(Parent::Collection(&collection_id)).await {
        Ok(rows) => axum::Json(serde_json::json!({ "items": rows })).into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn get_item<S>(State(vault): State<VaultCtx>, Path(id): Path<String>) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let store = match vault.items.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("items"),
    };
    match store.get_item(&id).await {
        Ok(Some(i)) => axum::Json(i).into_response(),
        Ok(None) => vault_err_to_response(VaultError::NotFound),
        Err(e) => vault_err_to_response(e),
    }
}

async fn update_item<S>(
    State(vault): State<VaultCtx>,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<CreateItemBody>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let store = match vault.items.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("items"),
    };
    let (ct, nonce) = match decode_pair(&body.ciphertext_b64, &body.nonce_b64) {
        Ok(p) => p,
        Err(e) => return vault_err_to_response(e),
    };
    match store
        .update_item(
            &id,
            &body.item_type,
            &body.name,
            &ct,
            &nonce,
            body.folder_id.as_deref(),
        )
        .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => vault_err_to_response(VaultError::NotFound),
        Err(e) => vault_err_to_response(e),
    }
}

async fn delete_item<S>(State(vault): State<VaultCtx>, Path(id): Path<String>) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let store = match vault.items.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("items"),
    };
    match store.delete_item(&id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => vault_err_to_response(VaultError::NotFound),
        Err(e) => vault_err_to_response(e),
    }
}

// ─────────────────────────────────────────────────────────────
// Folders
// ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateFolderBody {
    /// Either `vault_id` or `collection_id` (XOR). The handler enforces
    /// the XOR before forwarding to the store.
    vault_id: Option<String>,
    collection_id: Option<String>,
    parent_folder_id: Option<String>,
    name: String,
}

#[derive(Deserialize)]
struct RenameFolderBody {
    name: String,
}

async fn create_folder<S>(
    State(vault): State<VaultCtx>,
    axum::Json(body): axum::Json<CreateFolderBody>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let store = match vault.folders.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("folders"),
    };
    let parent = match (&body.vault_id, &body.collection_id) {
        (Some(v), None) => Parent::Vault(v),
        (None, Some(c)) => Parent::Collection(c),
        _ => {
            return vault_err_to_response(VaultError::Invalid(
                "exactly one of vault_id / collection_id must be set".into(),
            ));
        }
    };
    let id = new_id();
    match store
        .create_folder(&id, parent, body.parent_folder_id.as_deref(), &body.name)
        .await
    {
        Ok(f) => (StatusCode::CREATED, axum::Json(f)).into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn get_folder<S>(State(vault): State<VaultCtx>, Path(id): Path<String>) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let store = match vault.folders.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("folders"),
    };
    match store.get_folder(&id).await {
        Ok(Some(f)) => axum::Json(f).into_response(),
        Ok(None) => vault_err_to_response(VaultError::NotFound),
        Err(e) => vault_err_to_response(e),
    }
}

async fn rename_folder<S>(
    State(vault): State<VaultCtx>,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<RenameFolderBody>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let store = match vault.folders.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("folders"),
    };
    match store.rename_folder(&id, &body.name).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => vault_err_to_response(VaultError::NotFound),
        Err(e) => vault_err_to_response(e),
    }
}

async fn delete_folder<S>(State(vault): State<VaultCtx>, Path(id): Path<String>) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let store = match vault.folders.as_ref() {
        Some(s) => s.clone(),
        None => return unavailable("folders"),
    };
    match store.delete_folder(&id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => vault_err_to_response(VaultError::NotFound),
        Err(e) => vault_err_to_response(e),
    }
}

fn decode_pair(ct_b64: &str, nonce_b64: &str) -> crate::error::Result<(Vec<u8>, Vec<u8>)> {
    let ct = data_encoding::BASE64
        .decode(ct_b64.as_bytes())
        .map_err(|_| VaultError::Invalid("ciphertext_b64 is not valid base64".into()))?;
    let nonce = data_encoding::BASE64
        .decode(nonce_b64.as_bytes())
        .map_err(|_| VaultError::Invalid("nonce_b64 is not valid base64".into()))?;
    Ok((ct, nonce))
}
