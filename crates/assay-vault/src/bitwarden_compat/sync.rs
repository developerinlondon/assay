//! `GET /api/sync` — full vault dump in BW shape.
//!
//! BW clients call this after login + every periodic refresh. We
//! return the user's personal items as Ciphers + their folders,
//! all in the BW JSON wire format. Collections / orgs / policies /
//! sends are returned as empty arrays; Phase 7 doesn't yet wire
//! cross-org collections through the BW shim (collections live
//! under our own /api/v1/vault/collections/* surface for now).

use axum::Router;
use axum::extract::FromRef;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use assay_auth::AuthCtx;

use super::types::{Cipher, Folder, Profile, SyncResponse};
use crate::ctx::VaultCtx;
use crate::items::Parent;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    AuthCtx: FromRef<S>,
{
    Router::new().route("/api/sync", get(sync::<S>))
}

async fn sync<S>(
    axum::extract::State(vault): axum::extract::State<VaultCtx>,
    axum::extract::State(auth): axum::extract::State<AuthCtx>,
    headers: HeaderMap,
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
    let items_store = match vault.items.as_ref() {
        Some(s) => s.clone(),
        None => return super::service_unavailable("items"),
    };
    let folders_store = match vault.folders.as_ref() {
        Some(s) => s.clone(),
        None => return super::service_unavailable("folders"),
    };

    let pv_row = match pv.get_by_owner(&user_id).await {
        Ok(Some(v)) => v,
        Ok(None) => return super::not_found(),
        Err(e) => return super::vault_err(e),
    };
    let items = match items_store.list_items(Parent::Vault(&pv_row.id)).await {
        Ok(rows) => rows,
        Err(e) => return super::vault_err(e),
    };
    let folder_rows = match folders_store.list_folders(Parent::Vault(&pv_row.id)).await {
        Ok(rows) => rows,
        Err(e) => return super::vault_err(e),
    };

    let user = auth.users.get_user_by_id(&user_id).await.ok().flatten();
    let profile = match user {
        Some(u) => Profile {
            id: u.id,
            email: u.email.clone().unwrap_or_default(),
            email_verified: u.email_verified,
            name: u.display_name,
            premium: false,
            culture: "en-US".into(),
            key: None,
            private_key: None,
            security_stamp: "00000000".into(),
            object: "profile",
        },
        None => return super::not_found(),
    };

    let ciphers: Vec<Cipher> = items
        .into_iter()
        .map(|i| {
            // Round-trip whatever the client originally sent. The
            // ciphertext bytes are hex-encoded into the `Data` field
            // so a future client redownload sees the same blob shape.
            let data = serde_json::json!({
                "name": i.name,
                "ciphertext_b64": data_encoding::BASE64.encode(&i.ciphertext),
                "nonce_b64": data_encoding::BASE64.encode(&i.nonce),
            });
            Cipher {
                id: i.id,
                user_id: Some(user_id.clone()),
                organization_id: None,
                folder_id: i.folder_id,
                item_type: parse_item_type(&i.item_type),
                name: i.name,
                data: Some(data),
                login: None,
                secure_note: None,
                card: None,
                identity: None,
                favorite: false,
                revision_date: rfc3339(i.updated_at),
                object: "cipherDetails",
            }
        })
        .collect();

    let folders: Vec<Folder> = folder_rows
        .into_iter()
        .map(|f| Folder {
            id: f.id,
            name: f.name,
            revision_date: rfc3339(f.created_at),
            object: "folder",
        })
        .collect();

    axum::Json(SyncResponse {
        profile,
        folders,
        ciphers,
        collections: vec![],
        policies: vec![],
        sends: vec![],
        domains: serde_json::json!({}),
        object: "sync",
    })
    .into_response()
}

pub(super) fn parse_item_type(s: &str) -> i32 {
    match s {
        "login" => 1,
        "secureNote" | "secure_note" => 2,
        "card" => 3,
        "identity" => 4,
        "sshKey" | "ssh_key" => 5,
        _ => 1,
    }
}

pub(super) fn item_type_str(i: i32) -> &'static str {
    match i {
        1 => "login",
        2 => "secureNote",
        3 => "card",
        4 => "identity",
        5 => "sshKey",
        _ => "login",
    }
}

pub(super) fn rfc3339(epoch_secs: f64) -> String {
    let secs = epoch_secs as i64;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
