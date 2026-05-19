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
pub mod auth_params;
pub mod authorize;
pub mod binding;
pub mod consent;
pub mod discovery;
pub mod federation;
pub mod handlers;
pub mod introspect;
pub mod issuer_validation;
pub mod jwks;
pub mod revoke;
pub mod store;
pub mod token;
pub mod types;
pub mod upstreams_public;
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
    /// When `true` (legacy default), `upstream_callback` creates an
    /// `auth.users` row the first time it sees a new upstream subject.
    /// When `false`, the callback looks up by `email` and rejects with
    /// 403 if no row exists — the "invite-only" posture an operator
    /// gets by pre-populating `auth.users` via the admin API / the
    /// `/auth/users` sysops page.
    pub auto_provision: bool,
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
            // Default `true` to preserve historical library behaviour;
            // engine deployments set `false` via engine.toml for
            // invite-only posture.
            auto_provision: true,
        }
    }

    /// Toggle invite-only vs auto-provision at the federation callback.
    /// `false` → `auth.users` row must already exist (lookup by email).
    /// `true`  → callback creates the row on first sign-in (legacy).
    pub fn with_auto_provision(mut self, on: bool) -> Self {
        self.auto_provision = on;
        self
    }

    /// Replace the JWKS source. Engine boot calls this with the
    /// active backend pool so `/jwks.json` reads the live row set.
    pub fn with_jwks_source(mut self, src: JwksSource) -> Self {
        self.jwks_source = src;
        self
    }
}

/// Build the federation callback URL for an upstream provider.
/// Trims a trailing slash from `public_url` so a misconfigured
/// `server.public_url` (e.g. `https://auth.example.com/`) doesn't
/// produce a double-slash, which would break exact OIDC redirect_uri
/// matching.
pub fn upstream_callback_url(public_url: &url::Url, slug: &str) -> String {
    let base = public_url.as_str().trim_end_matches('/');
    format!("{base}/oidc/upstream/{slug}/callback")
}

/// Sync a single upstream provider row into the in-memory
/// [`crate::oidc::OidcRegistry`]. If the row is enabled, performs OIDC
/// discovery against the issuer and caches the client; if disabled,
/// removes any cached entry for that slug.
///
/// Reads `row.scopes` and `row.auth_params` directly. When `row.scopes`
/// is empty (typically because a row predates the V5 migration filling
/// in the default), falls back to [`crate::oidc::DEFAULT_UPSTREAM_SCOPES`].
pub async fn sync_upstream_to_registry(
    registry: &crate::oidc::OidcRegistry,
    row: &types::UpstreamProvider,
    public_url: &url::Url,
) {
    if row.enabled {
        let uri = match url::Url::parse(&upstream_callback_url(public_url, &row.slug)) {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("invalid callback URL for upstream {}: {e}", row.slug);
                return;
            }
        };
        let scopes = if row.scopes.is_empty() {
            crate::oidc::DEFAULT_UPSTREAM_SCOPES
                .iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            row.scopes.clone()
        };
        let provider = crate::oidc::UpstreamProvider {
            slug: row.slug.clone(),
            issuer: row.issuer.clone(),
            client_id: row.client_id.clone(),
            client_secret: row.client_secret.clone(),
            scopes,
            auth_params: row.auth_params.clone(),
        };
        if let Err(e) = registry.add(provider, uri).await {
            tracing::warn!("registry sync failed for upstream {}: {e}", row.slug);
        }
    } else {
        registry.remove(&row.slug);
    }
}

/// OIDC spec router — the OAuth2/OIDC well-known surface, mounted at
/// `/auth` by the engine binary. See route declarations below for the
/// authoritative path/handler list.
pub fn spec_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AuthCtx: FromRef<S>,
{
    Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(discovery::discovery_handler),
        )
        .route("/.well-known/jwks.json", get(jwks::jwks_handler))
        .route("/authorize", get(handlers::authorize_get))
        .route(
            "/authorize/consent",
            get(consent_preview).post(handlers::consent_post),
        )
        .route("/token", post(handlers::token_post))
        .route(
            "/userinfo",
            get(handlers::userinfo_get).post(handlers::userinfo_get),
        )
        .route("/revoke", post(handlers::revoke_post))
        .route("/introspect", post(handlers::introspect_post))
        .route("/logout", get(handlers::logout_get))
        .route("/oidc/upstream/{slug}/start", get(handlers::upstream_start))
        .route(
            "/oidc/upstream/{slug}/callback",
            get(handlers::upstream_callback),
        )
        // Public listing of enabled upstream IdPs — consumed by the
        // login landing in `assay-dashboard` to render upstream buttons
        // without an admin key. Returns only slug + display_name +
        // icon_url; secrets and disabled rows are filtered server-side.
        .route("/upstreams", get(upstreams_public::list_public))
}

/// OIDC admin router — operator-only CRUD for OIDC clients and
/// upstream federation providers, mounted under `/api/v1/engine/auth/`
/// (so paths land at `/api/v1/engine/auth/admin/oidc/...`). Each
/// handler enforces admin-key auth itself; see route declarations
/// below for the canonical surface.
pub fn admin_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AuthCtx: FromRef<S>,
    crate::state::AdminApiKeys: FromRef<S>,
{
    Router::new()
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

/// Backward-compat alias that returns the merged spec + admin
/// surface. Internal callers should pick the more specific router
/// ([`spec_router`] or [`admin_router`]); the engine binary uses both
/// directly.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AuthCtx: FromRef<S>,
    crate::state::AdminApiKeys: FromRef<S>,
{
    spec_router::<S>().merge(admin_router::<S>())
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
