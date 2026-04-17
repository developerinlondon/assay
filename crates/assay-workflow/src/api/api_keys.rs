//! API key management endpoints.
//!
//! Provides REST creation/listing/deletion of engine API keys as an
//! alternative to the `assay serve --generate-api-key` CLI subcommand.
//!
//! `POST /api/v1/api-keys` supports a client-supplied `label` and an
//! `idempotent` flag. When `idempotent = true` and a key with that label
//! already exists, the handler returns the existing record's metadata
//! *without* a plaintext — the plaintext was handed out at generation
//! time and is never retrievable again.
//!
//! The POST endpoint is intentionally callable **without authentication**
//! when the `api_keys` table is empty (see `api/auth.rs` middleware). This
//! is the first-ever-key bootstrap window: without it, a freshly deployed
//! server running in API-key or combined mode has no way to receive its
//! first credential. The window closes as soon as any key exists.
use std::sync::Arc;

use axum::extract::State;
use axum::routing::{delete, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::auth::{generate_api_key, hash_api_key, key_prefix};
use crate::api::workflows::AppError;
use crate::api::AppState;
use crate::store::{ApiKeyRecord, WorkflowStore};

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> {
    Router::new()
        .route("/api-keys", post(create_api_key).get(list_api_keys))
        .route("/api-keys/{prefix}", delete(revoke_api_key))
}

#[derive(Deserialize, ToSchema)]
pub struct CreateApiKeyRequest {
    /// Optional label to tag the key with. Labels can be arbitrary strings
    /// but are most useful when unique — combined with `idempotent=true`
    /// they let a caller provision a named key across reruns.
    #[serde(default)]
    pub label: Option<String>,

    /// If true AND a key with this `label` already exists, the handler
    /// returns `200 OK` with the existing record's metadata (no plaintext).
    /// If false (default) or no label is supplied, the handler always
    /// mints a fresh key and returns `201 Created` with the plaintext.
    #[serde(default)]
    pub idempotent: bool,
}

#[derive(Serialize, ToSchema)]
pub struct CreateApiKeyResponse {
    /// Plaintext API key. Only present on a fresh mint (`201 Created`).
    /// Never included when an existing key is returned idempotently.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plaintext: Option<String>,
    pub prefix: String,
    pub label: Option<String>,
    pub created_at: f64,
}

#[utoipa::path(
    post, path = "/api/v1/api-keys",
    tag = "api-keys",
    request_body = CreateApiKeyRequest,
    responses(
        (status = 201, description = "New API key minted", body = CreateApiKeyResponse),
        (status = 200, description = "Idempotent: existing key with this label returned (no plaintext)", body = CreateApiKeyResponse),
        (status = 500, description = "Internal error"),
    ),
)]
pub async fn create_api_key<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<(axum::http::StatusCode, Json<CreateApiKeyResponse>), AppError> {
    if req.idempotent
        && let Some(label) = req.label.as_deref()
        && let Some(existing) = state.engine.store().get_api_key_by_label(label).await?
    {
        return Ok((
            axum::http::StatusCode::OK,
            Json(CreateApiKeyResponse {
                plaintext: None,
                prefix: existing.prefix,
                label: existing.label,
                created_at: existing.created_at,
            }),
        ));
    }

    let plaintext = generate_api_key();
    let hash = hash_api_key(&plaintext);
    let prefix = key_prefix(&plaintext);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);

    state
        .engine
        .store()
        .create_api_key(&hash, &prefix, req.label.as_deref(), now)
        .await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(CreateApiKeyResponse {
            plaintext: Some(plaintext),
            prefix,
            label: req.label,
            created_at: now,
        }),
    ))
}

#[utoipa::path(
    get, path = "/api/v1/api-keys",
    tag = "api-keys",
    responses(
        (status = 200, description = "List of API key metadata (hashes never exposed)", body = Vec<ApiKeyRecord>),
    ),
)]
pub async fn list_api_keys<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
) -> Result<Json<Vec<ApiKeyRecord>>, AppError> {
    let keys = state.engine.store().list_api_keys().await?;
    Ok(Json(keys))
}

#[utoipa::path(
    delete, path = "/api/v1/api-keys/{prefix}",
    tag = "api-keys",
    params(("prefix" = String, Path, description = "Key prefix (e.g. assay_abcd1234...)")),
    responses(
        (status = 204, description = "Key revoked"),
        (status = 404, description = "No key with that prefix"),
    ),
)]
pub async fn revoke_api_key<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    axum::extract::Path(prefix): axum::extract::Path<String>,
) -> Result<axum::http::StatusCode, AppError> {
    let removed = state.engine.store().revoke_api_key(&prefix).await?;
    if removed {
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        Ok(axum::http::StatusCode::NOT_FOUND)
    }
}
