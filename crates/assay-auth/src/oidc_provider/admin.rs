//! Admin HTTP API for OIDC client + upstream provider management.
//!
//! Auth: every handler in this module requires a valid bearer token
//! from `auth.admin_api_keys`. The check happens at the handler entry
//! via [`require_admin`] — keeps the gating obvious and per-route
//! testable.
//!
//! Surface:
//!
//! - `GET    /admin/oidc/clients`
//! - `POST   /admin/oidc/clients` — returns the plaintext `client_secret` ONCE.
//! - `GET    /admin/oidc/clients/{id}`
//! - `PUT    /admin/oidc/clients/{id}`
//! - `DELETE /admin/oidc/clients/{id}`
//! - `POST   /admin/oidc/clients/{id}/rotate-secret` — new secret ONCE.
//!
//! - `GET    /admin/oidc/upstream`
//! - `POST   /admin/oidc/upstream`            → upsert by slug.
//! - `GET    /admin/oidc/upstream/{slug}`
//! - `DELETE /admin/oidc/upstream/{slug}`
//!
//! Choice (v0.2.0): admin auth is api-key only. Session-based admin
//! (Zanzibar role check) lands in v0.2.1; the trait surface already
//! supports it (the `AdminApiKeys` extractor would just become
//! `AdminAuth { keys, session }`).

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ctx::AuthCtx;
use crate::state::AdminApiKeys;

use super::types::{OidcClient, TokenAuthMethod, UpstreamProvider};

/// Auth + Zanzibar gate shared by every OIDC admin handler. Resolves a
/// [`crate::gate::Caller`] from the request, then enforces the
/// `auth#system#admin` role (same role as the cross-cutting admin
/// router — OIDC client/upstream CRUD is operator-level concern, not
/// per-tenant). Admin api-key callers bypass as break-glass.
pub(crate) async fn require_admin(
    headers: &HeaderMap,
    ctx: &AuthCtx,
    keys: &AdminApiKeys,
) -> Result<crate::gate::Caller, Box<Response>> {
    crate::gate::require_role_for(headers, ctx, keys, "auth", "system", "admin").await
}

// =====================================================================
//   /admin/oidc/clients
// =====================================================================

/// Body for `POST /admin/oidc/clients`. We accept the canonical
/// [`OidcClient`] shape minus `created_at` (stamped server-side) and
/// minus `client_secret_hash` (we mint it for confidential clients).
#[derive(Clone, Debug, Deserialize)]
pub struct CreateClientBody {
    pub client_id: Option<String>,
    pub redirect_uris: Vec<String>,
    pub name: String,
    pub logo_url: Option<String>,
    #[serde(default = "default_auth_method")]
    pub token_endpoint_auth_method: String,
    #[serde(default = "default_scopes")]
    pub default_scopes: Vec<String>,
    #[serde(default = "default_true")]
    pub require_consent: bool,
    #[serde(default = "default_grant_types")]
    pub grant_types: Vec<String>,
    #[serde(default = "default_response_types")]
    pub response_types: Vec<String>,
    #[serde(default = "default_true")]
    pub pkce_required: bool,
    pub backchannel_logout_uri: Option<String>,
}

fn default_auth_method() -> String {
    "client_secret_basic".to_string()
}
fn default_scopes() -> Vec<String> {
    vec!["openid".to_string()]
}
fn default_true() -> bool {
    true
}
fn default_grant_types() -> Vec<String> {
    vec![
        "authorization_code".to_string(),
        "refresh_token".to_string(),
    ]
}
fn default_response_types() -> Vec<String> {
    vec!["code".to_string()]
}

/// Returned ONCE on create / rotate-secret. The plaintext is never
/// readable again — operators MUST capture it from this response.
#[derive(Clone, Debug, Serialize)]
pub struct CreateClientResponse {
    pub client: OidcClient,
    /// Plaintext bearer for confidential clients. `None` for `none`
    /// (PKCE-only) clients.
    pub client_secret: Option<String>,
}

pub async fn create_client(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Json(body): Json<CreateClientBody>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => return svc_unavailable("oidc_provider not enabled"),
    };
    if body.redirect_uris.is_empty() {
        return bad_request("redirect_uris must be non-empty");
    }
    for u in &body.redirect_uris {
        if url::Url::parse(u).is_err() {
            return bad_request(&format!("redirect_uri {u:?} is not a URL"));
        }
    }
    let auth_method = match TokenAuthMethod::parse(&body.token_endpoint_auth_method) {
        Some(m) => m,
        None => {
            return bad_request(&format!(
                "unknown token_endpoint_auth_method {:?}",
                body.token_endpoint_auth_method
            ));
        }
    };
    let client_id = body.client_id.clone().unwrap_or_else(|| {
        format!(
            "ocl_{}",
            data_encoding::BASE64URL_NOPAD.encode(&random_bytes::<12>())
        )
    });
    let plaintext_secret = match auth_method {
        TokenAuthMethod::None => None,
        _ => Some(format!(
            "ocs_{}",
            data_encoding::BASE64URL_NOPAD.encode(&random_bytes::<24>())
        )),
    };
    let secret_hash = match &plaintext_secret {
        Some(s) => {
            let hasher = crate::password::PasswordHasher::default();
            match hasher.hash(s) {
                Ok(h) => Some(h),
                Err(e) => return server_error(&format!("hash secret: {e}")),
            }
        }
        None => None,
    };
    let client = OidcClient {
        client_id: client_id.clone(),
        client_secret_hash: secret_hash,
        redirect_uris: body.redirect_uris,
        name: body.name,
        logo_url: body.logo_url,
        token_endpoint_auth_method: auth_method,
        default_scopes: body.default_scopes,
        require_consent: body.require_consent,
        grant_types: body.grant_types,
        response_types: body.response_types,
        pkce_required: body.pkce_required,
        backchannel_logout_uri: body.backchannel_logout_uri,
        created_at: now_secs(),
    };
    if let Err(e) = provider.clients.create(&client).await {
        return server_error(&format!("persist client: {e}"));
    }
    (
        StatusCode::CREATED,
        Json(CreateClientResponse {
            client,
            client_secret: plaintext_secret,
        }),
    )
        .into_response()
}

pub async fn list_clients(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => return svc_unavailable("oidc_provider not enabled"),
    };
    match provider.clients.list().await {
        Ok(list) => (StatusCode::OK, Json(list)).into_response(),
        Err(e) => server_error(&format!("list clients: {e}")),
    }
}

pub async fn get_client(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => return svc_unavailable("oidc_provider not enabled"),
    };
    match provider.clients.get(&id).await {
        Ok(Some(c)) => (StatusCode::OK, Json(c)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown client_id"})),
        )
            .into_response(),
        Err(e) => server_error(&format!("get client: {e}")),
    }
}

/// Body for `PUT /admin/oidc/clients/{id}` — same shape as create
/// minus the auto-minted fields. Operators send the full record they
/// want persisted.
#[derive(Clone, Debug, Deserialize)]
pub struct UpdateClientBody {
    pub redirect_uris: Vec<String>,
    pub name: String,
    pub logo_url: Option<String>,
    pub token_endpoint_auth_method: String,
    pub default_scopes: Vec<String>,
    pub require_consent: bool,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    pub pkce_required: bool,
    pub backchannel_logout_uri: Option<String>,
}

pub async fn update_client(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<UpdateClientBody>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => return svc_unavailable("oidc_provider not enabled"),
    };
    let existing = match provider.clients.get(&id).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "unknown client_id"})),
            )
                .into_response();
        }
        Err(e) => return server_error(&format!("client lookup: {e}")),
    };
    let auth_method = match TokenAuthMethod::parse(&body.token_endpoint_auth_method) {
        Some(m) => m,
        None => {
            return bad_request(&format!(
                "unknown token_endpoint_auth_method {:?}",
                body.token_endpoint_auth_method
            ));
        }
    };
    let updated = OidcClient {
        client_id: existing.client_id,
        client_secret_hash: existing.client_secret_hash,
        redirect_uris: body.redirect_uris,
        name: body.name,
        logo_url: body.logo_url,
        token_endpoint_auth_method: auth_method,
        default_scopes: body.default_scopes,
        require_consent: body.require_consent,
        grant_types: body.grant_types,
        response_types: body.response_types,
        pkce_required: body.pkce_required,
        backchannel_logout_uri: body.backchannel_logout_uri,
        created_at: existing.created_at,
    };
    if let Err(e) = provider.clients.update(&updated).await {
        return server_error(&format!("update client: {e}"));
    }
    (StatusCode::OK, Json(updated)).into_response()
}

pub async fn delete_client(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => return svc_unavailable("oidc_provider not enabled"),
    };
    match provider.clients.delete(&id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown client_id"})),
        )
            .into_response(),
        Err(e) => server_error(&format!("delete client: {e}")),
    }
}

/// `POST /admin/oidc/clients/{id}/rotate-secret` — mints a fresh
/// client_secret, hashes it, persists it, returns the plaintext ONCE.
#[derive(Clone, Debug, Serialize)]
pub struct RotateSecretResponse {
    pub client_id: String,
    pub client_secret: String,
}

pub async fn rotate_client_secret(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => return svc_unavailable("oidc_provider not enabled"),
    };
    let plaintext = format!(
        "ocs_{}",
        data_encoding::BASE64URL_NOPAD.encode(&random_bytes::<24>())
    );
    let hasher = crate::password::PasswordHasher::default();
    let hash = match hasher.hash(&plaintext) {
        Ok(h) => h,
        Err(e) => return server_error(&format!("hash secret: {e}")),
    };
    match provider.clients.rotate_secret_hash(&id, &hash).await {
        Ok(true) => (
            StatusCode::OK,
            Json(RotateSecretResponse {
                client_id: id,
                client_secret: plaintext,
            }),
        )
            .into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown client_id"})),
        )
            .into_response(),
        Err(e) => server_error(&format!("rotate secret: {e}")),
    }
}

// =====================================================================
//   /admin/oidc/upstream
// =====================================================================

/// Body for the upsert path — `slug` is the natural key.
#[derive(Clone, Debug, Deserialize)]
pub struct UpstreamBody {
    pub slug: String,
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub display_name: String,
    pub icon_url: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

pub async fn upsert_upstream(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Json(body): Json<UpstreamBody>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => return svc_unavailable("oidc_provider not enabled"),
    };
    let row = UpstreamProvider {
        slug: body.slug,
        issuer: body.issuer,
        client_id: body.client_id,
        client_secret: body.client_secret,
        display_name: body.display_name,
        icon_url: body.icon_url,
        enabled: body.enabled,
    };
    if let Err(e) = provider.upstream.upsert(&row).await {
        return server_error(&format!("upsert upstream: {e}"));
    }
    (StatusCode::OK, Json(row)).into_response()
}

pub async fn list_upstream(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => return svc_unavailable("oidc_provider not enabled"),
    };
    match provider.upstream.list().await {
        Ok(list) => (StatusCode::OK, Json(list)).into_response(),
        Err(e) => server_error(&format!("list upstream: {e}")),
    }
}

pub async fn get_upstream(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => return svc_unavailable("oidc_provider not enabled"),
    };
    match provider.upstream.get(&slug).await {
        Ok(Some(u)) => (StatusCode::OK, Json(u)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown slug"})),
        )
            .into_response(),
        Err(e) => server_error(&format!("get upstream: {e}")),
    }
}

pub async fn delete_upstream(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => return svc_unavailable("oidc_provider not enabled"),
    };
    match provider.upstream.delete(&slug).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown slug"})),
        )
            .into_response(),
        Err(e) => server_error(&format!("delete upstream: {e}")),
    }
}

// =====================================================================
//   helpers
// =====================================================================

fn bad_request(msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": "invalid_request", "error_description": msg})),
    )
        .into_response()
}

fn server_error(msg: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "server_error", "error_description": msg})),
    )
        .into_response()
}

fn svc_unavailable(msg: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error": "service_unavailable", "error_description": msg})),
    )
        .into_response()
}

fn now_secs() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn random_bytes<const N: usize>() -> [u8; N] {
    use rand::RngCore;
    let mut buf = [0u8; N];
    rand::rng().fill_bytes(&mut buf);
    buf
}

// Admin-gate behaviour is covered in `crate::gate::tests` and the
// integration-test suite — `require_admin` here is a one-line wrapper
// over `gate::require_role_for`, so a per-handler test would just
// duplicate gate.rs's coverage.
