//! Transit HTTP handlers (plan 17 §S2).
//!
//! Wire shape — plaintext + ciphertext are base64 in JSON for binary
//! safety:
//!
//! ```text
//! POST /api/v1/vault/transit/keys/{name}             body: { algo? } -> 201
//! GET  /api/v1/vault/transit/keys                                   list
//! POST /api/v1/vault/transit/keys/{name}/rotate                     -> { version }
//! POST /api/v1/vault/transit/encrypt/{name}          body: { plaintext_b64 } -> { ciphertext }
//! POST /api/v1/vault/transit/decrypt/{name}          body: { ciphertext } -> { plaintext_b64 }
//! ```

use axum::Router;
use axum::extract::{FromRef, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};

use crate::ctx::VaultCtx;
use crate::error::VaultError;
use crate::router::vault_err_to_response;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    Router::new()
        .route("/transit/keys/{name}", post(create_key::<S>))
        .route("/transit/keys", get(list_keys::<S>))
        .route("/transit/keys/{name}/rotate", post(rotate::<S>))
        .route("/transit/encrypt/{name}", post(encrypt::<S>))
        .route("/transit/decrypt/{name}", post(decrypt::<S>))
}

#[derive(Deserialize, Default)]
struct CreateKeyBody {
    algo: Option<String>,
}

#[derive(Deserialize)]
struct EncryptBody {
    plaintext_b64: String,
}

#[derive(Serialize)]
struct EncryptResponse {
    ciphertext: String,
}

#[derive(Deserialize)]
struct DecryptBody {
    ciphertext: String,
}

#[derive(Serialize)]
struct DecryptResponse {
    plaintext_b64: String,
}

#[derive(Serialize)]
struct RotateResponse {
    name: String,
    version: i64,
}

async fn create_key<S>(
    State(vault): State<VaultCtx>,
    Path(name): Path<String>,
    body: Option<axum::Json<CreateKeyBody>>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let svc = match vault.transit.as_ref() {
        Some(t) => t,
        None => return service_unavailable("transit"),
    };
    let algo = body.and_then(|b| b.0.algo);
    match svc.create_key(&name, algo.as_deref()).await {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn list_keys<S>(State(vault): State<VaultCtx>) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let svc = match vault.transit.as_ref() {
        Some(t) => t,
        None => return service_unavailable("transit"),
    };
    match svc.list_keys().await {
        Ok(ks) => axum::Json(serde_json::json!({ "keys": ks })).into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn rotate<S>(State(vault): State<VaultCtx>, Path(name): Path<String>) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let svc = match vault.transit.as_ref() {
        Some(t) => t,
        None => return service_unavailable("transit"),
    };
    match svc.rotate(&name).await {
        Ok(version) => axum::Json(RotateResponse { name, version }).into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn encrypt<S>(
    State(vault): State<VaultCtx>,
    Path(name): Path<String>,
    axum::Json(body): axum::Json<EncryptBody>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let svc = match vault.transit.as_ref() {
        Some(t) => t,
        None => return service_unavailable("transit"),
    };
    let plaintext = match data_encoding::BASE64.decode(body.plaintext_b64.as_bytes()) {
        Ok(b) => b,
        Err(_) => {
            return vault_err_to_response(VaultError::Invalid(
                "plaintext_b64 is not valid base64".into(),
            ));
        }
    };
    match svc.encrypt(&name, &plaintext).await {
        Ok(ciphertext) => axum::Json(EncryptResponse { ciphertext }).into_response(),
        Err(e) => vault_err_to_response(e),
    }
}

async fn decrypt<S>(
    State(vault): State<VaultCtx>,
    Path(name): Path<String>,
    axum::Json(body): axum::Json<DecryptBody>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let svc = match vault.transit.as_ref() {
        Some(t) => t,
        None => return service_unavailable("transit"),
    };
    match svc.decrypt(&name, &body.ciphertext).await {
        Ok(plaintext) => axum::Json(DecryptResponse {
            plaintext_b64: data_encoding::BASE64.encode(&plaintext),
        })
        .into_response(),
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
