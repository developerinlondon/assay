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
//! - [`handlers`] — phase 8 — concrete axum handlers consuming AuthCtx.
//! - [`admin`] — phase 8 — admin HTTP API (clients/upstream CRUD).
//!
//! The crate-level [`OidcProviderConfig`] composes the subsystem stores +
//! issuer URL + JWT signing config so the AuthCtx carries one cohesive
//! value.

use std::sync::Arc;

use axum::Router;
use axum::extract::{FromRef, State};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use url::Url;

use crate::ctx::AuthCtx;

pub mod admin;
pub mod authorize;
pub mod consent;
pub mod discovery;
pub mod federation;
pub mod handlers;
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
    ///
    /// Long argument list is the cost of being explicit about which
    /// store backs each persistence concern (clients, upstream IdPs,
    /// auth codes, refresh tokens, sessions, consents, upstream-flow
    /// state). A builder/struct refactor was considered but rejected
    /// for now — the engine binary is the only caller and a one-shot
    /// `OidcProviderConfig::new(...)` reads cleanly there.
    #[allow(clippy::too_many_arguments)]
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

/// Mount the OIDC provider routes. Returns a [`Router`] generic over
/// any state `S` from which `AuthCtx` can be extracted via
/// `axum::extract::FromRef`. The top-level [`crate::router::router`]
/// merges this in when the `auth-oidc-provider` feature is on.
///
/// Routes:
///
/// - `GET /.well-known/openid-configuration`
/// - `GET /.well-known/jwks.json`
/// - `GET /authorize` — full handler (login redirect / consent / code mint).
/// - `POST /token` — full handler (auth code + refresh grant dispatch).
/// - `GET /userinfo` — bearer parse + JWT verify + scope-filtered claims.
/// - `POST /revoke` — RFC 7009 (refresh token marked revoked).
/// - `POST /introspect` — RFC 7662 (active/inactive response).
/// - `GET /logout` — session revoke + post_logout_redirect.
/// - `GET /authorize/consent` — render consent preview page.
/// - `POST /authorize/consent` — record consent + resume the flow.
/// - `GET /oidc/upstream/{slug}/start` — federation start.
/// - `GET /oidc/upstream/{slug}/callback` — federation completion.
/// - `POST /admin/oidc/clients[/{id}]` & friends — client + upstream
///   CRUD (admin-key gated).
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AuthCtx: FromRef<S>,
    crate::state::AdminApiKeys: FromRef<S>,
{
    Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(discovery::discovery_handler),
        )
        .route("/.well-known/jwks.json", get(jwks::jwks_handler))
        .route("/authorize", get(handlers::authorize_get))
        .route("/authorize/consent", get(consent_preview).post(handlers::consent_post))
        .route("/token", post(handlers::token_post))
        .route("/userinfo", get(handlers::userinfo_get).post(handlers::userinfo_get))
        .route("/revoke", post(handlers::revoke_post))
        .route("/introspect", post(handlers::introspect_post))
        .route("/logout", get(handlers::logout_get))
        .route(
            "/oidc/upstream/{slug}/start",
            get(handlers::upstream_start),
        )
        .route(
            "/oidc/upstream/{slug}/callback",
            get(handlers::upstream_callback),
        )
        // Admin routes — gated by api-key middleware in the handler.
        .route(
            "/admin/oidc/clients",
            get(admin::list_clients).post(admin::create_client),
        )
        .route(
            "/admin/oidc/clients/{id}",
            get(admin::get_client)
                .put(admin::update_client)
                .delete(admin::delete_client),
        )
        .route(
            "/admin/oidc/clients/{id}/rotate-secret",
            post(admin::rotate_client_secret),
        )
        .route(
            "/admin/oidc/upstream",
            get(admin::list_upstream).post(admin::upsert_upstream),
        )
        .route(
            "/admin/oidc/upstream/{slug}",
            get(admin::get_upstream).delete(admin::delete_upstream),
        )
}

/// Lightweight preview handler for the consent screen so the askama
/// template compiles + the route is reachable from a browser. Phase 8's
/// real consent flow is in [`handlers::consent_get`] — this stays as
/// the GET fallback that just renders a sample page (no real flow
/// state).
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
        // Identity FromRef: AuthCtx is its own parent state.
        let _: Router<crate::state::AuthCtxWithAdmin> = router();
    }
}
