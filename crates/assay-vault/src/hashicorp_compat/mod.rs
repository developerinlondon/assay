//! HashiCorp Vault / OpenBao **read-path** compatibility facade.
//!
//! Serves the Vault dialect — `X-Vault-Token` auth, `/v1/{mount}/data/{path}`
//! KV2 reads, Vault's response envelope — on top of [`crate::kv`], so ESO's
//! vault provider, ansible's `community.hashi_vault`, `vault kv get` and curl
//! reach assay-vault by repointing a URL. Full mapping, config, and consumer
//! examples: `docs/vault-hashicorp-compat.md`.
//!
//! ```text
//! GET  /v1/{mount}/data/{path}          latest version (?version=N for a specific one)
//! GET  /v1/{mount}/metadata/{path}      path-level metadata
//! LIST /v1/{mount}/metadata/{prefix}    immediate children (also GET …?list=true)
//! GET  /v1/sys/health                   liveness + seal state (unauthenticated)
//! ```
//!
//! Read-only by construction: no writes, no `sys/*` beyond health, no token
//! issuance, no auth mounts. Any other method on a facade route answers 405.
//!
//! The mount is a label, not a namespace — it names the one logical KV2 mount
//! this engine exposes and is stripped before the lookup, leaving the assay KV
//! path verbatim. Any other mount is a 404, so a typo cannot read a different
//! secret than the caller named.
//!
//! A Vault token IS an assay token: [`router`] takes the same mandatory `gate`
//! closure [`crate::router::vault_router`] does and pre-translates
//! `X-Vault-Token` into the bearer that gate already checks, so there is no
//! second token store and one enforcement point.
//!
//! Response field sets are copied from the KV2 spec
//! (<https://developer.hashicorp.com/vault/api-docs/secret/kv/kv-v2>).
//! Fields assay has no equivalent for are served at their Vault defaults, so a
//! client reading them sees a coherent document rather than a missing key.

use std::sync::Arc;

use axum::Router;
use axum::extract::{FromRef, Request};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::ctx::VaultCtx;

mod kv2;
mod sys;

/// Mount name served when the embedder doesn't configure one.
pub const DEFAULT_MOUNT: &str = "secrets";

const VAULT_TOKEN_HEADER: &str = "x-vault-token";

/// The single logical KV2 mount this facade answers for.
#[derive(Clone, Debug)]
pub struct Mount(Arc<str>);

impl Mount {
    pub fn new(name: impl AsRef<str>) -> Self {
        let name = name.as_ref().trim().trim_matches('/');
        if name.is_empty() {
            return Self::default();
        }
        Self(Arc::from(name))
    }

    pub fn name(&self) -> &str {
        &self.0
    }
}

impl Default for Mount {
    fn default() -> Self {
        Self(Arc::from(DEFAULT_MOUNT))
    }
}

/// Compose the compat facade, generic over a parent state from which
/// [`VaultCtx`] is extractable.
///
/// `gate` is the wire-boundary auth layer and is **non-optional** — an
/// unauthenticated facade is a compile error. `GET /v1/sys/health` sits
/// outside it because Vault's health endpoint is unauthenticated and k8s
/// probes plus ESO store validation hit it without a token.
///
/// Mount at the router root: Vault clients hardcode `/v1/…` and cannot be
/// told to use a prefix.
pub fn router<S, F>(mount: Mount, gate: F) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
    F: FnOnce(Router<S>) -> Router<S>,
{
    let gated = gate(kv2::router::<S>());
    gated
        .merge(sys::router::<S>())
        .layer(axum::Extension(mount))
        .layer(axum::middleware::from_fn(vault_error_envelope))
        .layer(axum::middleware::from_fn(adopt_vault_token))
}

/// Applied outermost so it runs before the gate. A caller-set
/// `Authorization` header wins, keeping plain bearer callers working.
async fn adopt_vault_token(mut request: Request, next: Next) -> Response {
    if !request.headers().contains_key(header::AUTHORIZATION)
        && let Some(token) = request.headers().get(VAULT_TOKEN_HEADER)
        && let Ok(token) = token.to_str()
        && let Ok(value) = format!("Bearer {}", token.trim()).parse()
    {
        request.headers_mut().insert(header::AUTHORIZATION, value);
    }
    next.run(request).await
}

/// Restate gate and routing rejections in Vault's vocabulary: Vault answers
/// 403 for a token that is missing as well as one that is wrong, and every
/// error carries an `errors` array where assay's own surface carries `error`.
async fn vault_error_envelope(request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            errors(StatusCode::FORBIDDEN, &["permission denied"])
        }
        StatusCode::METHOD_NOT_ALLOWED => {
            errors(StatusCode::METHOD_NOT_ALLOWED, &["unsupported operation"])
        }
        _ => response,
    }
}

pub(crate) fn errors(status: StatusCode, messages: &[&str]) -> Response {
    (status, axum::Json(json!({ "errors": messages }))).into_response()
}

/// An empty `errors` array is the shape Vault uses for "no such path".
pub(crate) fn not_found() -> Response {
    errors(StatusCode::NOT_FOUND, &[])
}

/// The wrapper every Vault read response carries; only `data` varies across
/// the endpoints served here.
pub(crate) fn envelope(data: Value) -> Value {
    json!({
        "request_id": uuid::Uuid::new_v4().to_string(),
        "lease_id": "",
        "renewable": false,
        "lease_duration": 0,
        "data": data,
        "wrap_info": null,
        "warnings": null,
        "auth": null,
    })
}

/// Vault renders timestamps as RFC 3339 with nanosecond precision and a `Z`
/// offset; assay stores unix seconds as `f64`.
pub(crate) fn rfc3339(secs: f64) -> String {
    let nanos = ((secs - secs.trunc()) * 1e9).round() as u32;
    chrono::DateTime::from_timestamp(secs.trunc() as i64, nanos.min(999_999_999))
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))
        .unwrap_or_default()
}

pub(crate) fn vault_error(e: crate::error::VaultError) -> Response {
    use crate::error::VaultError as E;
    match e {
        E::NotFound => not_found(),
        E::Sealed => errors(StatusCode::SERVICE_UNAVAILABLE, &["Vault is sealed"]),
        E::Forbidden => errors(StatusCode::FORBIDDEN, &["permission denied"]),
        E::Invalid(msg) => errors(StatusCode::BAD_REQUEST, &[msg.as_str()]),
        other => errors(
            StatusCode::INTERNAL_SERVER_ERROR,
            &[other.to_string().as_str()],
        ),
    }
}

/// Vault has no vocabulary for "this mount exists but is not wired", so the
/// facade says it plainly rather than pretending the path is missing.
pub(crate) fn kv_unconfigured() -> Response {
    errors(
        StatusCode::SERVICE_UNAVAILABLE,
        &["vault kv surface not configured"],
    )
}

/// Strips the decoration Vault clients put around key paths: a leading slash
/// from a hand-written URL, a trailing one from a list prefix.
pub(crate) fn normalize_path(path: &str) -> &str {
    path.trim_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_falls_back_to_the_default_when_blank() {
        assert_eq!(Mount::new("  ").name(), DEFAULT_MOUNT);
        assert_eq!(Mount::new("/").name(), DEFAULT_MOUNT);
        assert_eq!(Mount::default().name(), "secrets");
    }

    #[test]
    fn mount_ignores_slashes_a_config_file_may_carry() {
        assert_eq!(Mount::new("/kv/").name(), "kv");
    }

    #[test]
    fn timestamps_render_the_way_vault_renders_them() {
        assert_eq!(rfc3339(0.0), "1970-01-01T00:00:00.000000000Z");
        assert!(rfc3339(1_755_000_000.5).starts_with("2025-08-12T"));
        assert!(rfc3339(1_755_000_000.5).ends_with("Z"));
    }

    #[test]
    fn list_prefixes_and_leading_slashes_resolve_to_the_same_key() {
        assert_eq!(normalize_path("/platform/postgres"), "platform/postgres");
        assert_eq!(normalize_path("platform/"), "platform");
        assert_eq!(normalize_path(""), "");
    }
}
