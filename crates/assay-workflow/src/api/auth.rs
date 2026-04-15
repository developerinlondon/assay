use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::api::AppState;
use crate::store::WorkflowStore;

/// Auth configuration — determines which mode the engine runs in.
#[derive(Clone, Debug, Default)]
pub enum AuthMode {
    /// No authentication — all requests allowed (dev mode).
    #[default]
    NoAuth,
    /// API key authentication — Bearer token validated against hashed keys in DB.
    ApiKey,
    /// JWT/OIDC — validate Bearer JWT against a JWKS endpoint.
    Jwt {
        issuer: String,
        audience: Option<String>,
    },
}

/// Axum middleware that enforces authentication based on the configured mode.
pub async fn auth_middleware<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    request: Request,
    next: Next,
) -> Response {
    match &state.auth_mode {
        AuthMode::NoAuth => next.run(request).await,
        AuthMode::ApiKey => validate_api_key(state, request, next).await,
        AuthMode::Jwt { issuer, audience } => {
            validate_jwt(issuer, audience.as_deref(), request, next).await
        }
    }
}

async fn validate_api_key<S: WorkflowStore>(
    state: Arc<AppState<S>>,
    request: Request,
    next: Next,
) -> Response {
    let token = match extract_bearer(&request) {
        Some(t) => t,
        None => return auth_error("Missing Authorization: Bearer <api-key>"),
    };

    let hash = hash_api_key(token);
    match state.engine.store().validate_api_key(&hash).await {
        Ok(true) => next.run(request).await,
        Ok(false) => {
            warn!("Invalid API key (prefix: {}...)", &token[..8.min(token.len())]);
            auth_error("Invalid API key")
        }
        Err(e) => {
            warn!("API key validation error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "auth check failed"})),
            )
                .into_response()
        }
    }
}

async fn validate_jwt(
    issuer: &str,
    audience: Option<&str>,
    request: Request,
    next: Next,
) -> Response {
    let token = match extract_bearer(&request) {
        Some(t) => t,
        None => return auth_error("Missing Authorization: Bearer <jwt>"),
    };

    // Decode header to check algorithm
    let header = match jsonwebtoken::decode_header(token) {
        Ok(h) => h,
        Err(e) => {
            warn!("Invalid JWT header: {e}");
            return auth_error("Invalid JWT");
        }
    };

    // Build validation
    let mut validation = jsonwebtoken::Validation::new(header.alg);
    validation.set_issuer(&[issuer]);
    if let Some(aud) = audience {
        validation.set_audience(&[aud]);
    } else {
        validation.validate_aud = false;
    }

    // Decode without signature verification, then validate claims manually.
    // Full JWKS signature validation requires async key fetching — will be added
    // when we implement the JWKS cache.
    // TODO: fetch JWKS from {issuer}/.well-known/openid-configuration
    let token_data =
        match jsonwebtoken::dangerous::insecure_decode::<serde_json::Value>(token) {
            Ok(data) => data,
            Err(e) => {
                warn!("JWT decode failed: {e}");
                return auth_error("Invalid JWT");
            }
        };

    // Validate issuer
    if let Some(iss) = token_data.claims.get("iss").and_then(|v| v.as_str()) {
        if iss != issuer {
            warn!("JWT issuer mismatch: expected {issuer}, got {iss}");
            return auth_error("JWT issuer mismatch");
        }
    } else {
        return auth_error("JWT missing issuer claim");
    }

    // Validate expiry
    if let Some(exp) = token_data.claims.get("exp").and_then(|v| v.as_f64()) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        if now > exp {
            return auth_error("JWT expired");
        }
    }

    // Validate audience if configured
    if let Some(expected_aud) = audience {
        let aud_match = token_data
            .claims
            .get("aud")
            .map(|v| match v {
                serde_json::Value::String(s) => s == expected_aud,
                serde_json::Value::Array(arr) => {
                    arr.iter().any(|a| a.as_str() == Some(expected_aud))
                }
                _ => false,
            })
            .unwrap_or(false);
        if !aud_match {
            return auth_error("JWT audience mismatch");
        }
    }

    next.run(request).await
}

fn extract_bearer(request: &Request) -> Option<&str> {
    request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

fn auth_error(msg: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": msg})),
    )
        .into_response()
}

/// Hash an API key with SHA-256 for storage/lookup.
pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    data_encoding::HEXLOWER.encode(&hasher.finalize())
}

/// Generate a new random API key (32 bytes, hex-encoded).
pub fn generate_api_key() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::rng().random();
    format!("assay_{}", data_encoding::HEXLOWER.encode(&bytes))
}

/// Extract the prefix (first 8 chars after "assay_") for display.
pub fn key_prefix(key: &str) -> String {
    let stripped = key.strip_prefix("assay_").unwrap_or(key);
    format!("assay_{}...", &stripped[..8.min(stripped.len())])
}
