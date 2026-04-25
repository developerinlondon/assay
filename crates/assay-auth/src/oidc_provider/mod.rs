//! OIDC provider — full Hydra-equivalent identity provider.
//!
//! Module shape mirrors the rest of `assay-auth`'s directory modules
//! (`store/`, `zanzibar/`):
//!
//! - [`types`] — POD records mirroring the V4 DDL (clients, codes,
//!   refresh, sessions, consent, upstream state).
//! - [`store`] — trait + PG/SQLite implementations for each row table.
//! - [`discovery`] — `/.well-known/openid-configuration`.
//! - [`jwks`] — `/.well-known/jwks.json`.
//! - [`authorize`] — `/authorize` request validation + code minting.
//! - [`token`] — `/token` (auth code + refresh) + JWT claim builders.
//! - [`userinfo`] — `/userinfo` claim filtering.
//! - [`consent`] — askama-rendered consent screen.
//! - [`revoke`] — RFC 7009 token revocation.
//! - [`introspect`] — RFC 7662 token introspection.
//! - [`federation`] — upstream login start/complete (Google/Apple/etc).
//!
//! The crate-level [`OidcProviderConfig`] composes the subsystem stores
//! + issuer URL + JWT signing config so the AuthCtx carries one
//! cohesive value.

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Json};
use axum::routing::{get, post};
use serde_json::json;
use url::Url;

use crate::ctx::AuthCtx;

pub mod authorize;
pub mod consent;
pub mod discovery;
pub mod federation;
pub mod introspect;
pub mod jwks;
pub mod revoke;
pub mod store;
pub mod token;
pub mod types;
pub mod userinfo;

pub use store::{
    OidcClientStore, OidcCodeStore, OidcConsentStore, OidcRefreshStore, OidcSessionStore,
    OidcUpstreamStateStore, OidcUpstreamStore,
};
pub use types::{
    AuthorizationCode, ConsentGrant, OidcClient, OidcSession, RefreshToken, TokenAuthMethod,
    UpstreamLoginState, UpstreamProvider,
};

#[cfg(feature = "backend-postgres")]
pub use store::{
    PostgresOidcClientStore, PostgresOidcCodeStore, PostgresOidcConsentStore,
    PostgresOidcRefreshStore, PostgresOidcSessionStore, PostgresOidcUpstreamStateStore,
    PostgresOidcUpstreamStore,
};
#[cfg(feature = "backend-sqlite")]
pub use store::{
    SqliteOidcClientStore, SqliteOidcCodeStore, SqliteOidcConsentStore, SqliteOidcRefreshStore,
    SqliteOidcSessionStore, SqliteOidcUpstreamStateStore, SqliteOidcUpstreamStore,
};

/// Source the JWKS endpoint reads from. PG / SQLite back the V4 jwks
/// table; `Memory` is for tests + ephemeral configurations.
#[derive(Clone)]
pub enum JwksSource {
    #[cfg(feature = "backend-postgres")]
    Postgres(sqlx::PgPool),
    #[cfg(feature = "backend-sqlite")]
    Sqlite(sqlx::SqlitePool),
    Memory(Vec<serde_json::Value>),
}

/// Composed configuration for the OIDC provider. Cheap to clone — every
/// store / pool inside is `Arc` already.
#[derive(Clone)]
pub struct OidcProviderConfig {
    /// Stable issuer URL — appears in `iss` claims and discovery doc.
    pub issuer: String,
    /// Public engine URL — used to derive default redirect targets
    /// (login page, federation callback).
    pub public_url: Url,
    pub clients: Arc<dyn OidcClientStore>,
    pub upstream: Arc<dyn OidcUpstreamStore>,
    pub codes: Arc<dyn OidcCodeStore>,
    pub refresh: Arc<dyn OidcRefreshStore>,
    pub sessions: Arc<dyn OidcSessionStore>,
    pub consents: Arc<dyn OidcConsentStore>,
    pub upstream_states: Arc<dyn OidcUpstreamStateStore>,
    pub jwks_source: JwksSource,
}

impl OidcProviderConfig {
    /// Build a provider config carrying the given stores. The default
    /// JWKS source is `Memory(vec![])` — engine boot replaces it with
    /// a backend-specific pool.
    pub fn new(
        issuer: impl Into<String>,
        public_url: Url,
        clients: Arc<dyn OidcClientStore>,
        upstream: Arc<dyn OidcUpstreamStore>,
        codes: Arc<dyn OidcCodeStore>,
        refresh: Arc<dyn OidcRefreshStore>,
        sessions: Arc<dyn OidcSessionStore>,
        consents: Arc<dyn OidcConsentStore>,
        upstream_states: Arc<dyn OidcUpstreamStateStore>,
    ) -> Self {
        Self {
            issuer: issuer.into(),
            public_url,
            clients,
            upstream,
            codes,
            refresh,
            sessions,
            consents,
            upstream_states,
            jwks_source: JwksSource::Memory(Vec::new()),
        }
    }

    /// Replace the JWKS source. Engine boot calls this with the
    /// active backend pool so `/jwks.json` reads the live row set.
    pub fn with_jwks_source(mut self, src: JwksSource) -> Self {
        self.jwks_source = src;
        self
    }
}

/// Mount the OIDC provider routes. Returns a [`Router`] over [`AuthCtx`]
/// that the top-level [`crate::router::router`] merges in when the
/// `auth-oidc-provider` feature is on.
///
/// Routes:
///
/// - `GET /.well-known/openid-configuration`
/// - `GET /.well-known/jwks.json`
/// - `GET /authorize` — (placeholder body) returns 501; full handler is
///   wired in phase 8 alongside the login UI.
/// - `POST /token` — (placeholder)
/// - `GET /userinfo` — (placeholder)
/// - `POST /revoke` — (placeholder)
/// - `POST /introspect` — (placeholder)
/// - `GET /logout` — (placeholder)
///
/// The placeholders carry the full helper logic (validation, claim
/// build) under the hood; the HTTP wiring just lacks the handler glue
/// that materialises the resolved AuthCtx (session resolution, JWT
/// signing) — phase 8 supplies it. Until then, hitting them returns
/// `501 Not Implemented` so an OIDC client probing the discovery doc
/// fails fast rather than hanging.
pub fn router() -> Router<AuthCtx> {
    Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(discovery::discovery_handler),
        )
        .route("/.well-known/jwks.json", get(jwks::jwks_handler))
        .route("/authorize", get(not_yet_implemented))
        .route("/token", post(not_yet_implemented))
        .route("/userinfo", get(not_yet_implemented))
        .route("/revoke", post(not_yet_implemented))
        .route("/introspect", post(not_yet_implemented))
        .route("/logout", get(not_yet_implemented))
        .route("/authorize/consent", get(consent_preview))
}

/// Sentinel handler — 501 with a JSON-shaped error body. Used for
/// every route whose final HTTP wiring lands in phase 8 (it needs the
/// engine-side login UI + AuthCtx resolution helpers that don't live
/// in `assay-auth` yet).
async fn not_yet_implemented() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({"error": "not_implemented", "error_description": "OIDC provider HTTP handlers wired in phase 8"})),
    )
}

/// Lightweight preview handler for the consent screen so the askama
/// template compiles + the route is reachable from a browser. Phase 8
/// replaces this with the real flow that resolves the in-progress
/// authorize request.
async fn consent_preview(State(ctx): State<AuthCtx>) -> impl IntoResponse {
    let issuer = ctx
        .oidc_provider
        .as_ref()
        .map(|p| p.issuer.clone())
        .unwrap_or_default();
    let scopes = vec![
        "openid".to_string(),
        "email".to_string(),
        "profile".to_string(),
    ];
    let page = consent::ConsentPage {
        client_name: "Example consumer app",
        issuer: &issuer,
        scopes: &scopes,
        csrf_token: "csrf_preview",
        resume_token: "resume_preview",
    };
    Html(page.render_html())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke-test that the discovery + JWKS handler types tag together
    /// — handler functions compile + bind to AuthCtx.
    #[test]
    fn router_constructs() {
        let _ = router();
    }
}
