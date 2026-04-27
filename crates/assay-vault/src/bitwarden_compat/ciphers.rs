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

/// Serialise the BW cipher's type-specific block into the bytes that
/// land in `vault.items.ciphertext`. The server stores the JSON
/// representation of (notes, login, secure_note, card, identity,
/// ssh_key) as one blob; nonce is empty (BW does its own encryption
/// — there's no server-side AEAD on these items, the type-specific
/// `encString`-format strings inside are already pre-encrypted by the
/// client).
fn pack_cipher_blob(input: &CipherInput) -> Vec<u8> {
    serde_json::to_vec(input).unwrap_or_default()
}

/// Reverse of [`pack_cipher_blob`] — read the stored JSON back into
/// the per-type fields BW clients expect on /sync. Robust to legacy
/// rows that hold opaque bytes (returns null for the structured
/// fields and lets the client fall back to its previous state).
fn unpack_cipher_blob(bytes: &[u8]) -> CipherInput {
    serde_json::from_slice(bytes).unwrap_or(CipherInput {
        folder_id: None,
        item_type: 1,
        name: String::new(),
        notes: None,
        favorite: false,
        login: None,
        secure_note: None,
        card: None,
        identity: None,
        ssh_key: None,
    })
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
    let blob = pack_cipher_blob(&input);
    let nonce: Vec<u8> = Vec::new();
    let item = match items
        .create_item(
            &id,
            Parent::Vault(&pv_row.id),
            input.folder_id.as_deref(),
            item_type_str(input.item_type),
            &input.name,
            &blob,
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
            notes: input.notes.clone(),
            login: input.login.clone(),
            secure_note: input.secure_note.clone(),
            card: input.card.clone(),
            identity: input.identity.clone(),
            ssh_key: input.ssh_key.clone(),
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
    let unpacked = unpack_cipher_blob(&item.ciphertext);
    let cipher = Cipher {
        id: item.id,
        user_id: Some(user_id),
        organization_id: None,
        folder_id: item.folder_id,
        item_type: parse_item_type(&item.item_type),
        name: item.name,
        notes: unpacked.notes,
        login: unpacked.login,
        secure_note: unpacked.secure_note,
        card: unpacked.card,
        identity: unpacked.identity,
        ssh_key: unpacked.ssh_key,
        favorite: unpacked.favorite,
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
    let blob = pack_cipher_blob(&input);
    let nonce: Vec<u8> = Vec::new();
    let updated = match items
        .update_item(
            &id,
            item_type_str(input.item_type),
            &input.name,
            &blob,
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
