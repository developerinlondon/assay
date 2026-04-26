//! BW `/api/ciphers/*` — CRUD against vault.items inside the user's
//! personal vault.

use axum::Router;
use axum::extract::{FromRef, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

use assay_auth::AuthCtx;

use super::sync::{item_type_str, parse_item_type, rfc3339};
use super::types::{Cipher, CipherInput};
use crate::ctx::VaultCtx;
use crate::items::Parent;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AuthCtx: FromRef<S>,
{
    Router::new()
        .route("/api/ciphers", post(create::<S>))
        .route(
            "/api/ciphers/{id}",
            get(get_one::<S>).put(update::<S>).delete(delete_one::<S>),
        )
}

fn ciphertext_pair(input: &CipherInput) -> (Vec<u8>, Vec<u8>) {
    // BW clients pre-encrypt cipher fields client-side; the bytes
    // arrive in the `Data` field of the input. We decode that into
    // ciphertext + nonce.
    let data = input.data.as_ref();
    let ct_b64 = data
        .and_then(|d| d.get("ciphertext_b64").and_then(|v| v.as_str()))
        .unwrap_or("");
    let nonce_b64 = data
        .and_then(|d| d.get("nonce_b64").and_then(|v| v.as_str()))
        .unwrap_or("");
    let ct = data_encoding::BASE64
        .decode(ct_b64.as_bytes())
        .unwrap_or_default();
    let nonce = data_encoding::BASE64
        .decode(nonce_b64.as_bytes())
        .unwrap_or_else(|_| vec![0u8; 12]);
    (ct, nonce)
}

async fn create<S>(
    State(vault): State<VaultCtx>,
    State(auth): State<AuthCtx>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<CipherInput>,
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
    let items = match vault.items.as_ref() {
        Some(s) => s.clone(),
        None => return super::service_unavailable("items"),
    };

    let pv_row = match pv.get_by_owner(&user_id).await {
        Ok(Some(v)) => v,
        Ok(None) => return super::not_found(),
        Err(e) => return super::vault_err(e),
    };

    let id = uuid::Uuid::now_v7().to_string();
    let (ct, nonce) = ciphertext_pair(&input);
    let item = match items
        .create_item(
            &id,
            Parent::Vault(&pv_row.id),
            input.folder_id.as_deref(),
            item_type_str(input.item_type),
            &input.name,
            &ct,
            &nonce,
        )
        .await
    {
        Ok(item) => item,
        Err(e) => return super::vault_err(e),
    };

    (
        StatusCode::CREATED,
        axum::Json(Cipher {
            id: item.id,
            user_id: Some(user_id),
            organization_id: None,
            folder_id: item.folder_id,
            item_type: input.item_type,
            name: item.name,
            data: input.data.clone(),
            login: input.login.clone(),
            secure_note: input.secure_note.clone(),
            card: input.card.clone(),
            identity: input.identity.clone(),
            favorite: input.favorite,
            revision_date: rfc3339(item.updated_at),
            object: "cipher",
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
    let user_id = match super::extract_user_id(&auth, &headers).await {
        Ok(uid) => uid,
        Err(r) => return r,
    };
    let items = match vault.items.as_ref() {
        Some(s) => s.clone(),
        None => return super::service_unavailable("items"),
    };
    let item = match items.get_item(&id).await {
        Ok(Some(i)) => i,
        Ok(None) => return super::not_found(),
        Err(e) => return super::vault_err(e),
    };
    let cipher = Cipher {
        id: item.id,
        user_id: Some(user_id),
        organization_id: None,
        folder_id: item.folder_id,
        item_type: parse_item_type(&item.item_type),
        name: item.name,
        data: Some(serde_json::json!({
            "ciphertext_b64": data_encoding::BASE64.encode(&item.ciphertext),
            "nonce_b64": data_encoding::BASE64.encode(&item.nonce),
        })),
        login: None,
        secure_note: None,
        card: None,
        identity: None,
        favorite: false,
        revision_date: rfc3339(item.updated_at),
        object: "cipher",
    };
    axum::Json(cipher).into_response()
}

async fn update<S>(
    State(vault): State<VaultCtx>,
    State(auth): State<AuthCtx>,
    headers: HeaderMap,
    Path(id): Path<String>,
    axum::Json(input): axum::Json<CipherInput>,
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
    let items = match vault.items.as_ref() {
        Some(s) => s.clone(),
        None => return super::service_unavailable("items"),
    };
    let (ct, nonce) = ciphertext_pair(&input);
    let updated = match items
        .update_item(
            &id,
            item_type_str(input.item_type),
            &input.name,
            &ct,
            &nonce,
            input.folder_id.as_deref(),
        )
        .await
    {
        Ok(true) => true,
        Ok(false) => return super::not_found(),
        Err(e) => return super::vault_err(e),
    };
    let _ = updated;
    StatusCode::NO_CONTENT.into_response()
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
    let items = match vault.items.as_ref() {
        Some(s) => s.clone(),
        None => return super::service_unavailable("items"),
    };
    match items.delete_item(&id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => super::not_found(),
        Err(e) => super::vault_err(e),
    }
}
