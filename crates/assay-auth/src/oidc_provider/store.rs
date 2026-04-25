//! Storage traits + PG/SQLite implementations for the OIDC provider.
//!
//! Two trait families:
//!
//! - [`OidcClientStore`] — CRUD over `auth.oidc_clients`.
//! - [`OidcUpstreamStore`] — CRUD over `auth.upstream_providers`.
//!
//! Plus the concrete row stores:
//!
//! - [`OidcCodeStore`] — issue / consume `auth.oidc_authorization_codes`.
//! - [`OidcRefreshStore`] — write / verify / revoke
//!   `auth.oidc_refresh_tokens`.
//! - [`OidcSessionStore`] — `auth.oidc_sessions` lookups for the SSO
//!   registry + back-channel logout fan-out.
//! - [`OidcConsentStore`] — per-(user, client) consent grants.
//! - [`OidcUpstreamStateStore`] — short-lived federation login state.
//!
//! All trait methods return `anyhow::Result<…>` so a backend can surface
//! its native error verbatim. Handlers translate to [`crate::Error`] at
//! the boundary.

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;

use super::types::{
    AuthorizationCode, ConsentGrant, OidcClient, OidcSession, RefreshToken, TokenAuthMethod,
    UpstreamLoginState, UpstreamProvider,
};

#[async_trait]
pub trait OidcClientStore: Send + Sync + 'static {
    async fn create(&self, client: &OidcClient) -> Result<()>;
    async fn get(&self, client_id: &str) -> Result<Option<OidcClient>>;
    async fn list(&self) -> Result<Vec<OidcClient>>;
    async fn update(&self, client: &OidcClient) -> Result<()>;
    async fn delete(&self, client_id: &str) -> Result<bool>;
    /// Replace the client_secret_hash. Returns Ok(false) if no row matched.
    async fn rotate_secret_hash(&self, client_id: &str, new_hash: &str) -> Result<bool>;
}

#[async_trait]
pub trait OidcUpstreamStore: Send + Sync + 'static {
    async fn upsert(&self, provider: &UpstreamProvider) -> Result<()>;
    async fn get(&self, slug: &str) -> Result<Option<UpstreamProvider>>;
    async fn list(&self) -> Result<Vec<UpstreamProvider>>;
    async fn delete(&self, slug: &str) -> Result<bool>;
}

#[async_trait]
pub trait OidcCodeStore: Send + Sync + 'static {
    async fn create(&self, code: &AuthorizationCode) -> Result<()>;
    /// Atomic consume — UPDATE … WHERE consumed = FALSE. If 0 rows are
    /// affected the code was either missing or already consumed; the
    /// caller treats both as `invalid_grant`. Returns the row's pre-
    /// consume snapshot when the consume succeeded.
    async fn consume(&self, code: &str) -> Result<Option<AuthorizationCode>>;
}

#[async_trait]
pub trait OidcRefreshStore: Send + Sync + 'static {
    async fn create(&self, token: &RefreshToken) -> Result<()>;
    async fn get(&self, token_hash: &str) -> Result<Option<RefreshToken>>;
    async fn revoke(&self, token_hash: &str) -> Result<bool>;
    /// Revoke every refresh token belonging to `user_id` — the replay-
    /// detection nuke per OAuth 2.1.
    async fn revoke_for_user(&self, user_id: &str) -> Result<u64>;
}

#[async_trait]
pub trait OidcSessionStore: Send + Sync + 'static {
    async fn create(&self, session: &OidcSession) -> Result<()>;
    async fn get(&self, sid: &str) -> Result<Option<OidcSession>>;
    /// Every SSO session row tied to a single assay session — used by
    /// `/logout` to fan out back-channel logout.
    async fn list_by_assay_session(&self, assay_session_id: &str) -> Result<Vec<OidcSession>>;
    async fn delete(&self, sid: &str) -> Result<bool>;
    async fn delete_by_assay_session(&self, assay_session_id: &str) -> Result<u64>;
}

#[async_trait]
pub trait OidcConsentStore: Send + Sync + 'static {
    async fn upsert(&self, grant: &ConsentGrant) -> Result<()>;
    async fn get(&self, user_id: &str, client_id: &str) -> Result<Option<ConsentGrant>>;
    async fn delete(&self, user_id: &str, client_id: &str) -> Result<bool>;
}

#[async_trait]
pub trait OidcUpstreamStateStore: Send + Sync + 'static {
    async fn create(&self, state: &UpstreamLoginState) -> Result<()>;
    /// Atomically delete and return — single use.
    async fn take(&self, state: &str) -> Result<Option<UpstreamLoginState>>;
}

// =====================================================================
//   POSTGRES
// =====================================================================

#[cfg(feature = "backend-postgres")]
mod pg {
    use super::*;
    use sqlx::{PgPool, Row};

    fn parse_json_array(s: &str) -> Vec<String> {
        serde_json::from_str(s).unwrap_or_default()
    }

    fn encode_json_array(v: &[String]) -> String {
        serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string())
    }

    fn map_client_row(row: sqlx::postgres::PgRow) -> OidcClient {
        let auth_method: String = row.get("token_endpoint_auth_method");
        OidcClient {
            client_id: row.get("client_id"),
            client_secret_hash: row.get("client_secret_hash"),
            redirect_uris: parse_json_array(&row.get::<String, _>("redirect_uris")),
            name: row.get("name"),
            logo_url: row.get("logo_url"),
            token_endpoint_auth_method: TokenAuthMethod::parse(&auth_method)
                .unwrap_or(TokenAuthMethod::ClientSecretBasic),
            default_scopes: parse_json_array(&row.get::<String, _>("default_scopes")),
            require_consent: row.get("require_consent"),
            grant_types: parse_json_array(&row.get::<String, _>("grant_types")),
            response_types: parse_json_array(&row.get::<String, _>("response_types")),
            pkce_required: row.get("pkce_required"),
            backchannel_logout_uri: row.get("backchannel_logout_uri"),
            created_at: row.get("created_at"),
        }
    }

    /// Postgres-backed [`OidcClientStore`].
    #[derive(Clone)]
    pub struct PostgresOidcClientStore {
        pool: PgPool,
    }

    impl PostgresOidcClientStore {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
        pub fn into_dyn(self) -> Arc<dyn OidcClientStore> {
            Arc::new(self)
        }
    }

    #[async_trait]
    impl OidcClientStore for PostgresOidcClientStore {
        async fn create(&self, c: &OidcClient) -> Result<()> {
            sqlx::query(
                "INSERT INTO auth.oidc_clients
                    (client_id, client_secret_hash, redirect_uris, name, logo_url,
                     token_endpoint_auth_method, default_scopes, require_consent,
                     grant_types, response_types, pkce_required,
                     backchannel_logout_uri, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            )
            .bind(&c.client_id)
            .bind(&c.client_secret_hash)
            .bind(encode_json_array(&c.redirect_uris))
            .bind(&c.name)
            .bind(&c.logo_url)
            .bind(c.token_endpoint_auth_method.as_str())
            .bind(encode_json_array(&c.default_scopes))
            .bind(c.require_consent)
            .bind(encode_json_array(&c.grant_types))
            .bind(encode_json_array(&c.response_types))
            .bind(c.pkce_required)
            .bind(&c.backchannel_logout_uri)
            .bind(c.created_at)
            .execute(&self.pool)
            .await
            .context("auth.oidc_clients insert")?;
            Ok(())
        }

        async fn get(&self, client_id: &str) -> Result<Option<OidcClient>> {
            let row = sqlx::query(
                "SELECT * FROM auth.oidc_clients WHERE client_id = $1",
            )
            .bind(client_id)
            .fetch_optional(&self.pool)
            .await
            .context("auth.oidc_clients select")?;
            Ok(row.map(map_client_row))
        }

        async fn list(&self) -> Result<Vec<OidcClient>> {
            let rows = sqlx::query("SELECT * FROM auth.oidc_clients ORDER BY created_at")
                .fetch_all(&self.pool)
                .await
                .context("auth.oidc_clients list")?;
            Ok(rows.into_iter().map(map_client_row).collect())
        }

        async fn update(&self, c: &OidcClient) -> Result<()> {
            sqlx::query(
                "UPDATE auth.oidc_clients SET
                    client_secret_hash = $2,
                    redirect_uris = $3,
                    name = $4,
                    logo_url = $5,
                    token_endpoint_auth_method = $6,
                    default_scopes = $7,
                    require_consent = $8,
                    grant_types = $9,
                    response_types = $10,
                    pkce_required = $11,
                    backchannel_logout_uri = $12
                 WHERE client_id = $1",
            )
            .bind(&c.client_id)
            .bind(&c.client_secret_hash)
            .bind(encode_json_array(&c.redirect_uris))
            .bind(&c.name)
            .bind(&c.logo_url)
            .bind(c.token_endpoint_auth_method.as_str())
            .bind(encode_json_array(&c.default_scopes))
            .bind(c.require_consent)
            .bind(encode_json_array(&c.grant_types))
            .bind(encode_json_array(&c.response_types))
            .bind(c.pkce_required)
            .bind(&c.backchannel_logout_uri)
            .execute(&self.pool)
            .await
            .context("auth.oidc_clients update")?;
            Ok(())
        }

        async fn delete(&self, client_id: &str) -> Result<bool> {
            let r = sqlx::query("DELETE FROM auth.oidc_clients WHERE client_id = $1")
                .bind(client_id)
                .execute(&self.pool)
                .await
                .context("auth.oidc_clients delete")?;
            Ok(r.rows_affected() > 0)
        }

        async fn rotate_secret_hash(&self, client_id: &str, new_hash: &str) -> Result<bool> {
            let r = sqlx::query(
                "UPDATE auth.oidc_clients SET client_secret_hash = $2 WHERE client_id = $1",
            )
            .bind(client_id)
            .bind(new_hash)
            .execute(&self.pool)
            .await
            .context("auth.oidc_clients rotate_secret_hash")?;
            Ok(r.rows_affected() > 0)
        }
    }

    fn map_upstream_row(row: sqlx::postgres::PgRow) -> UpstreamProvider {
        UpstreamProvider {
            slug: row.get("slug"),
            issuer: row.get("issuer"),
            client_id: row.get("client_id"),
            client_secret: row.get("client_secret"),
            display_name: row.get("display_name"),
            icon_url: row.get("icon_url"),
            enabled: row.get("enabled"),
        }
    }

    #[derive(Clone)]
    pub struct PostgresOidcUpstreamStore {
        pool: PgPool,
    }

    impl PostgresOidcUpstreamStore {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
        pub fn into_dyn(self) -> Arc<dyn OidcUpstreamStore> {
            Arc::new(self)
        }
    }

    #[async_trait]
    impl OidcUpstreamStore for PostgresOidcUpstreamStore {
        async fn upsert(&self, p: &UpstreamProvider) -> Result<()> {
            sqlx::query(
                "INSERT INTO auth.upstream_providers
                    (slug, issuer, client_id, client_secret, display_name, icon_url, enabled)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (slug) DO UPDATE SET
                    issuer = EXCLUDED.issuer,
                    client_id = EXCLUDED.client_id,
                    client_secret = EXCLUDED.client_secret,
                    display_name = EXCLUDED.display_name,
                    icon_url = EXCLUDED.icon_url,
                    enabled = EXCLUDED.enabled",
            )
            .bind(&p.slug)
            .bind(&p.issuer)
            .bind(&p.client_id)
            .bind(&p.client_secret)
            .bind(&p.display_name)
            .bind(&p.icon_url)
            .bind(p.enabled)
            .execute(&self.pool)
            .await
            .context("auth.upstream_providers upsert")?;
            Ok(())
        }

        async fn get(&self, slug: &str) -> Result<Option<UpstreamProvider>> {
            let row = sqlx::query("SELECT * FROM auth.upstream_providers WHERE slug = $1")
                .bind(slug)
                .fetch_optional(&self.pool)
                .await
                .context("auth.upstream_providers select")?;
            Ok(row.map(map_upstream_row))
        }

        async fn list(&self) -> Result<Vec<UpstreamProvider>> {
            let rows = sqlx::query("SELECT * FROM auth.upstream_providers ORDER BY slug")
                .fetch_all(&self.pool)
                .await
                .context("auth.upstream_providers list")?;
            Ok(rows.into_iter().map(map_upstream_row).collect())
        }

        async fn delete(&self, slug: &str) -> Result<bool> {
            let r = sqlx::query("DELETE FROM auth.upstream_providers WHERE slug = $1")
                .bind(slug)
                .execute(&self.pool)
                .await
                .context("auth.upstream_providers delete")?;
            Ok(r.rows_affected() > 0)
        }
    }

    fn map_code_row(row: sqlx::postgres::PgRow) -> AuthorizationCode {
        AuthorizationCode {
            code: row.get("code"),
            client_id: row.get("client_id"),
            user_id: row.get("user_id"),
            redirect_uri: row.get("redirect_uri"),
            scopes: parse_json_array(&row.get::<String, _>("scopes")),
            code_challenge: row.get("code_challenge"),
            code_challenge_method: row.get("code_challenge_method"),
            nonce: row.get("nonce"),
            state: row.get("state"),
            issued_at: row.get("issued_at"),
            expires_at: row.get("expires_at"),
            consumed: row.get("consumed"),
        }
    }

    #[derive(Clone)]
    pub struct PostgresOidcCodeStore {
        pool: PgPool,
    }
    impl PostgresOidcCodeStore {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
        pub fn into_dyn(self) -> Arc<dyn OidcCodeStore> {
            Arc::new(self)
        }
    }

    #[async_trait]
    impl OidcCodeStore for PostgresOidcCodeStore {
        async fn create(&self, c: &AuthorizationCode) -> Result<()> {
            sqlx::query(
                "INSERT INTO auth.oidc_authorization_codes
                    (code, client_id, user_id, redirect_uri, scopes,
                     code_challenge, code_challenge_method, nonce, state,
                     issued_at, expires_at, consumed)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            )
            .bind(&c.code)
            .bind(&c.client_id)
            .bind(&c.user_id)
            .bind(&c.redirect_uri)
            .bind(encode_json_array(&c.scopes))
            .bind(&c.code_challenge)
            .bind(&c.code_challenge_method)
            .bind(&c.nonce)
            .bind(&c.state)
            .bind(c.issued_at)
            .bind(c.expires_at)
            .bind(c.consumed)
            .execute(&self.pool)
            .await
            .context("auth.oidc_authorization_codes insert")?;
            Ok(())
        }

        async fn consume(&self, code: &str) -> Result<Option<AuthorizationCode>> {
            // RETURNING semantics — fetch the row at the same time we
            // mark it consumed. The `consumed = FALSE` predicate is the
            // single-use guarantee.
            let row = sqlx::query(
                "UPDATE auth.oidc_authorization_codes
                    SET consumed = TRUE
                    WHERE code = $1 AND consumed = FALSE
                    RETURNING *",
            )
            .bind(code)
            .fetch_optional(&self.pool)
            .await
            .context("auth.oidc_authorization_codes consume")?;
            Ok(row.map(map_code_row))
        }
    }

    fn map_refresh_row(row: sqlx::postgres::PgRow) -> RefreshToken {
        RefreshToken {
            token_hash: row.get("token_hash"),
            client_id: row.get("client_id"),
            user_id: row.get("user_id"),
            scopes: parse_json_array(&row.get::<String, _>("scopes")),
            issued_at: row.get("issued_at"),
            expires_at: row.get("expires_at"),
            revoked: row.get("revoked"),
        }
    }

    #[derive(Clone)]
    pub struct PostgresOidcRefreshStore {
        pool: PgPool,
    }
    impl PostgresOidcRefreshStore {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
        pub fn into_dyn(self) -> Arc<dyn OidcRefreshStore> {
            Arc::new(self)
        }
    }

    #[async_trait]
    impl OidcRefreshStore for PostgresOidcRefreshStore {
        async fn create(&self, t: &RefreshToken) -> Result<()> {
            sqlx::query(
                "INSERT INTO auth.oidc_refresh_tokens
                    (token_hash, client_id, user_id, scopes,
                     issued_at, expires_at, revoked)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(&t.token_hash)
            .bind(&t.client_id)
            .bind(&t.user_id)
            .bind(encode_json_array(&t.scopes))
            .bind(t.issued_at)
            .bind(t.expires_at)
            .bind(t.revoked)
            .execute(&self.pool)
            .await
            .context("auth.oidc_refresh_tokens insert")?;
            Ok(())
        }

        async fn get(&self, token_hash: &str) -> Result<Option<RefreshToken>> {
            let row = sqlx::query(
                "SELECT * FROM auth.oidc_refresh_tokens WHERE token_hash = $1",
            )
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .context("auth.oidc_refresh_tokens select")?;
            Ok(row.map(map_refresh_row))
        }

        async fn revoke(&self, token_hash: &str) -> Result<bool> {
            let r = sqlx::query(
                "UPDATE auth.oidc_refresh_tokens SET revoked = TRUE WHERE token_hash = $1",
            )
            .bind(token_hash)
            .execute(&self.pool)
            .await
            .context("auth.oidc_refresh_tokens revoke")?;
            Ok(r.rows_affected() > 0)
        }

        async fn revoke_for_user(&self, user_id: &str) -> Result<u64> {
            let r = sqlx::query(
                "UPDATE auth.oidc_refresh_tokens SET revoked = TRUE WHERE user_id = $1",
            )
            .bind(user_id)
            .execute(&self.pool)
            .await
            .context("auth.oidc_refresh_tokens revoke_for_user")?;
            Ok(r.rows_affected())
        }
    }

    fn map_session_row(row: sqlx::postgres::PgRow) -> OidcSession {
        OidcSession {
            sid: row.get("sid"),
            user_id: row.get("user_id"),
            client_id: row.get("client_id"),
            assay_session_id: row.get("assay_session_id"),
            issued_at: row.get("issued_at"),
            backchannel_logout_uri: row.get("backchannel_logout_uri"),
        }
    }

    #[derive(Clone)]
    pub struct PostgresOidcSessionStore {
        pool: PgPool,
    }
    impl PostgresOidcSessionStore {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
        pub fn into_dyn(self) -> Arc<dyn OidcSessionStore> {
            Arc::new(self)
        }
    }

    #[async_trait]
    impl OidcSessionStore for PostgresOidcSessionStore {
        async fn create(&self, s: &OidcSession) -> Result<()> {
            sqlx::query(
                "INSERT INTO auth.oidc_sessions
                    (sid, user_id, client_id, assay_session_id, issued_at, backchannel_logout_uri)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(&s.sid)
            .bind(&s.user_id)
            .bind(&s.client_id)
            .bind(&s.assay_session_id)
            .bind(s.issued_at)
            .bind(&s.backchannel_logout_uri)
            .execute(&self.pool)
            .await
            .context("auth.oidc_sessions insert")?;
            Ok(())
        }

        async fn get(&self, sid: &str) -> Result<Option<OidcSession>> {
            let row = sqlx::query("SELECT * FROM auth.oidc_sessions WHERE sid = $1")
                .bind(sid)
                .fetch_optional(&self.pool)
                .await
                .context("auth.oidc_sessions get")?;
            Ok(row.map(map_session_row))
        }

        async fn list_by_assay_session(&self, assay_session_id: &str) -> Result<Vec<OidcSession>> {
            let rows = sqlx::query(
                "SELECT * FROM auth.oidc_sessions WHERE assay_session_id = $1",
            )
            .bind(assay_session_id)
            .fetch_all(&self.pool)
            .await
            .context("auth.oidc_sessions list_by_assay_session")?;
            Ok(rows.into_iter().map(map_session_row).collect())
        }

        async fn delete(&self, sid: &str) -> Result<bool> {
            let r = sqlx::query("DELETE FROM auth.oidc_sessions WHERE sid = $1")
                .bind(sid)
                .execute(&self.pool)
                .await
                .context("auth.oidc_sessions delete")?;
            Ok(r.rows_affected() > 0)
        }

        async fn delete_by_assay_session(&self, assay_session_id: &str) -> Result<u64> {
            let r = sqlx::query(
                "DELETE FROM auth.oidc_sessions WHERE assay_session_id = $1",
            )
            .bind(assay_session_id)
            .execute(&self.pool)
            .await
            .context("auth.oidc_sessions delete_by_assay_session")?;
            Ok(r.rows_affected())
        }
    }

    fn map_consent_row(row: sqlx::postgres::PgRow) -> ConsentGrant {
        ConsentGrant {
            user_id: row.get("user_id"),
            client_id: row.get("client_id"),
            scopes: parse_json_array(&row.get::<String, _>("scopes")),
            granted_at: row.get("granted_at"),
        }
    }

    #[derive(Clone)]
    pub struct PostgresOidcConsentStore {
        pool: PgPool,
    }
    impl PostgresOidcConsentStore {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
        pub fn into_dyn(self) -> Arc<dyn OidcConsentStore> {
            Arc::new(self)
        }
    }

    #[async_trait]
    impl OidcConsentStore for PostgresOidcConsentStore {
        async fn upsert(&self, g: &ConsentGrant) -> Result<()> {
            sqlx::query(
                "INSERT INTO auth.oidc_consents (user_id, client_id, scopes, granted_at)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (user_id, client_id) DO UPDATE
                     SET scopes = EXCLUDED.scopes,
                         granted_at = EXCLUDED.granted_at",
            )
            .bind(&g.user_id)
            .bind(&g.client_id)
            .bind(encode_json_array(&g.scopes))
            .bind(g.granted_at)
            .execute(&self.pool)
            .await
            .context("auth.oidc_consents upsert")?;
            Ok(())
        }

        async fn get(&self, user_id: &str, client_id: &str) -> Result<Option<ConsentGrant>> {
            let row = sqlx::query(
                "SELECT * FROM auth.oidc_consents WHERE user_id = $1 AND client_id = $2",
            )
            .bind(user_id)
            .bind(client_id)
            .fetch_optional(&self.pool)
            .await
            .context("auth.oidc_consents get")?;
            Ok(row.map(map_consent_row))
        }

        async fn delete(&self, user_id: &str, client_id: &str) -> Result<bool> {
            let r = sqlx::query(
                "DELETE FROM auth.oidc_consents WHERE user_id = $1 AND client_id = $2",
            )
            .bind(user_id)
            .bind(client_id)
            .execute(&self.pool)
            .await
            .context("auth.oidc_consents delete")?;
            Ok(r.rows_affected() > 0)
        }
    }

    fn map_upstream_state_row(row: sqlx::postgres::PgRow) -> UpstreamLoginState {
        UpstreamLoginState {
            state: row.get("state"),
            provider_slug: row.get("provider_slug"),
            nonce: row.get("nonce"),
            pkce_verifier: row.get("pkce_verifier"),
            return_to: row.get("return_to"),
            created_at: row.get("created_at"),
            expires_at: row.get("expires_at"),
        }
    }

    #[derive(Clone)]
    pub struct PostgresOidcUpstreamStateStore {
        pool: PgPool,
    }
    impl PostgresOidcUpstreamStateStore {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
        pub fn into_dyn(self) -> Arc<dyn OidcUpstreamStateStore> {
            Arc::new(self)
        }
    }

    #[async_trait]
    impl OidcUpstreamStateStore for PostgresOidcUpstreamStateStore {
        async fn create(&self, s: &UpstreamLoginState) -> Result<()> {
            sqlx::query(
                "INSERT INTO auth.oidc_upstream_states
                    (state, provider_slug, nonce, pkce_verifier, return_to,
                     created_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(&s.state)
            .bind(&s.provider_slug)
            .bind(&s.nonce)
            .bind(&s.pkce_verifier)
            .bind(&s.return_to)
            .bind(s.created_at)
            .bind(s.expires_at)
            .execute(&self.pool)
            .await
            .context("auth.oidc_upstream_states insert")?;
            Ok(())
        }

        async fn take(&self, state: &str) -> Result<Option<UpstreamLoginState>> {
            // Atomic delete-and-return — single-use semantic.
            let row = sqlx::query(
                "DELETE FROM auth.oidc_upstream_states WHERE state = $1 RETURNING *",
            )
            .bind(state)
            .fetch_optional(&self.pool)
            .await
            .context("auth.oidc_upstream_states take")?;
            Ok(row.map(map_upstream_state_row))
        }
    }
}

#[cfg(feature = "backend-postgres")]
pub use pg::{
    PostgresOidcClientStore, PostgresOidcCodeStore, PostgresOidcConsentStore,
    PostgresOidcRefreshStore, PostgresOidcSessionStore, PostgresOidcUpstreamStateStore,
    PostgresOidcUpstreamStore,
};

// =====================================================================
//   SQLITE
// =====================================================================

#[cfg(feature = "backend-sqlite")]
mod sqlite_impl {
    use super::*;
    use sqlx::{Row, SqlitePool};

    fn parse_json_array(s: &str) -> Vec<String> {
        serde_json::from_str(s).unwrap_or_default()
    }
    fn encode_json_array(v: &[String]) -> String {
        serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string())
    }
    fn b(v: bool) -> i64 {
        if v { 1 } else { 0 }
    }
    fn ub(v: i64) -> bool {
        v != 0
    }

    fn map_client_row(row: sqlx::sqlite::SqliteRow) -> OidcClient {
        let auth_method: String = row.get("token_endpoint_auth_method");
        OidcClient {
            client_id: row.get("client_id"),
            client_secret_hash: row.get("client_secret_hash"),
            redirect_uris: parse_json_array(&row.get::<String, _>("redirect_uris")),
            name: row.get("name"),
            logo_url: row.get("logo_url"),
            token_endpoint_auth_method: TokenAuthMethod::parse(&auth_method)
                .unwrap_or(TokenAuthMethod::ClientSecretBasic),
            default_scopes: parse_json_array(&row.get::<String, _>("default_scopes")),
            require_consent: ub(row.get("require_consent")),
            grant_types: parse_json_array(&row.get::<String, _>("grant_types")),
            response_types: parse_json_array(&row.get::<String, _>("response_types")),
            pkce_required: ub(row.get("pkce_required")),
            backchannel_logout_uri: row.get("backchannel_logout_uri"),
            created_at: row.get("created_at"),
        }
    }

    #[derive(Clone)]
    pub struct SqliteOidcClientStore {
        pool: SqlitePool,
    }
    impl SqliteOidcClientStore {
        pub fn new(pool: SqlitePool) -> Self {
            Self { pool }
        }
        pub fn into_dyn(self) -> Arc<dyn OidcClientStore> {
            Arc::new(self)
        }
    }

    #[async_trait]
    impl OidcClientStore for SqliteOidcClientStore {
        async fn create(&self, c: &OidcClient) -> Result<()> {
            sqlx::query(
                "INSERT INTO auth.oidc_clients
                    (client_id, client_secret_hash, redirect_uris, name, logo_url,
                     token_endpoint_auth_method, default_scopes, require_consent,
                     grant_types, response_types, pkce_required,
                     backchannel_logout_uri, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&c.client_id)
            .bind(&c.client_secret_hash)
            .bind(encode_json_array(&c.redirect_uris))
            .bind(&c.name)
            .bind(&c.logo_url)
            .bind(c.token_endpoint_auth_method.as_str())
            .bind(encode_json_array(&c.default_scopes))
            .bind(b(c.require_consent))
            .bind(encode_json_array(&c.grant_types))
            .bind(encode_json_array(&c.response_types))
            .bind(b(c.pkce_required))
            .bind(&c.backchannel_logout_uri)
            .bind(c.created_at)
            .execute(&self.pool)
            .await
            .context("auth.oidc_clients insert")?;
            Ok(())
        }

        async fn get(&self, client_id: &str) -> Result<Option<OidcClient>> {
            let row = sqlx::query("SELECT * FROM auth.oidc_clients WHERE client_id = ?")
                .bind(client_id)
                .fetch_optional(&self.pool)
                .await
                .context("auth.oidc_clients get")?;
            Ok(row.map(map_client_row))
        }

        async fn list(&self) -> Result<Vec<OidcClient>> {
            let rows = sqlx::query("SELECT * FROM auth.oidc_clients ORDER BY created_at")
                .fetch_all(&self.pool)
                .await
                .context("auth.oidc_clients list")?;
            Ok(rows.into_iter().map(map_client_row).collect())
        }

        async fn update(&self, c: &OidcClient) -> Result<()> {
            sqlx::query(
                "UPDATE auth.oidc_clients SET
                    client_secret_hash = ?,
                    redirect_uris = ?,
                    name = ?,
                    logo_url = ?,
                    token_endpoint_auth_method = ?,
                    default_scopes = ?,
                    require_consent = ?,
                    grant_types = ?,
                    response_types = ?,
                    pkce_required = ?,
                    backchannel_logout_uri = ?
                 WHERE client_id = ?",
            )
            .bind(&c.client_secret_hash)
            .bind(encode_json_array(&c.redirect_uris))
            .bind(&c.name)
            .bind(&c.logo_url)
            .bind(c.token_endpoint_auth_method.as_str())
            .bind(encode_json_array(&c.default_scopes))
            .bind(b(c.require_consent))
            .bind(encode_json_array(&c.grant_types))
            .bind(encode_json_array(&c.response_types))
            .bind(b(c.pkce_required))
            .bind(&c.backchannel_logout_uri)
            .bind(&c.client_id)
            .execute(&self.pool)
            .await
            .context("auth.oidc_clients update")?;
            Ok(())
        }

        async fn delete(&self, client_id: &str) -> Result<bool> {
            let r = sqlx::query("DELETE FROM auth.oidc_clients WHERE client_id = ?")
                .bind(client_id)
                .execute(&self.pool)
                .await
                .context("auth.oidc_clients delete")?;
            Ok(r.rows_affected() > 0)
        }

        async fn rotate_secret_hash(&self, client_id: &str, new_hash: &str) -> Result<bool> {
            let r = sqlx::query(
                "UPDATE auth.oidc_clients SET client_secret_hash = ? WHERE client_id = ?",
            )
            .bind(new_hash)
            .bind(client_id)
            .execute(&self.pool)
            .await
            .context("auth.oidc_clients rotate_secret_hash")?;
            Ok(r.rows_affected() > 0)
        }
    }

    fn map_upstream_row(row: sqlx::sqlite::SqliteRow) -> UpstreamProvider {
        UpstreamProvider {
            slug: row.get("slug"),
            issuer: row.get("issuer"),
            client_id: row.get("client_id"),
            client_secret: row.get("client_secret"),
            display_name: row.get("display_name"),
            icon_url: row.get("icon_url"),
            enabled: ub(row.get("enabled")),
        }
    }

    #[derive(Clone)]
    pub struct SqliteOidcUpstreamStore {
        pool: SqlitePool,
    }
    impl SqliteOidcUpstreamStore {
        pub fn new(pool: SqlitePool) -> Self {
            Self { pool }
        }
        pub fn into_dyn(self) -> Arc<dyn OidcUpstreamStore> {
            Arc::new(self)
        }
    }

    #[async_trait]
    impl OidcUpstreamStore for SqliteOidcUpstreamStore {
        async fn upsert(&self, p: &UpstreamProvider) -> Result<()> {
            sqlx::query(
                "INSERT INTO auth.upstream_providers
                    (slug, issuer, client_id, client_secret, display_name, icon_url, enabled)
                 VALUES (?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (slug) DO UPDATE SET
                    issuer = excluded.issuer,
                    client_id = excluded.client_id,
                    client_secret = excluded.client_secret,
                    display_name = excluded.display_name,
                    icon_url = excluded.icon_url,
                    enabled = excluded.enabled",
            )
            .bind(&p.slug)
            .bind(&p.issuer)
            .bind(&p.client_id)
            .bind(&p.client_secret)
            .bind(&p.display_name)
            .bind(&p.icon_url)
            .bind(b(p.enabled))
            .execute(&self.pool)
            .await
            .context("auth.upstream_providers upsert")?;
            Ok(())
        }

        async fn get(&self, slug: &str) -> Result<Option<UpstreamProvider>> {
            let row = sqlx::query("SELECT * FROM auth.upstream_providers WHERE slug = ?")
                .bind(slug)
                .fetch_optional(&self.pool)
                .await
                .context("auth.upstream_providers get")?;
            Ok(row.map(map_upstream_row))
        }

        async fn list(&self) -> Result<Vec<UpstreamProvider>> {
            let rows = sqlx::query("SELECT * FROM auth.upstream_providers ORDER BY slug")
                .fetch_all(&self.pool)
                .await
                .context("auth.upstream_providers list")?;
            Ok(rows.into_iter().map(map_upstream_row).collect())
        }

        async fn delete(&self, slug: &str) -> Result<bool> {
            let r = sqlx::query("DELETE FROM auth.upstream_providers WHERE slug = ?")
                .bind(slug)
                .execute(&self.pool)
                .await
                .context("auth.upstream_providers delete")?;
            Ok(r.rows_affected() > 0)
        }
    }

    fn map_code_row(row: sqlx::sqlite::SqliteRow) -> AuthorizationCode {
        AuthorizationCode {
            code: row.get("code"),
            client_id: row.get("client_id"),
            user_id: row.get("user_id"),
            redirect_uri: row.get("redirect_uri"),
            scopes: parse_json_array(&row.get::<String, _>("scopes")),
            code_challenge: row.get("code_challenge"),
            code_challenge_method: row.get("code_challenge_method"),
            nonce: row.get("nonce"),
            state: row.get("state"),
            issued_at: row.get("issued_at"),
            expires_at: row.get("expires_at"),
            consumed: ub(row.get("consumed")),
        }
    }

    #[derive(Clone)]
    pub struct SqliteOidcCodeStore {
        pool: SqlitePool,
    }
    impl SqliteOidcCodeStore {
        pub fn new(pool: SqlitePool) -> Self {
            Self { pool }
        }
        pub fn into_dyn(self) -> Arc<dyn OidcCodeStore> {
            Arc::new(self)
        }
    }

    #[async_trait]
    impl OidcCodeStore for SqliteOidcCodeStore {
        async fn create(&self, c: &AuthorizationCode) -> Result<()> {
            sqlx::query(
                "INSERT INTO auth.oidc_authorization_codes
                    (code, client_id, user_id, redirect_uri, scopes,
                     code_challenge, code_challenge_method, nonce, state,
                     issued_at, expires_at, consumed)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&c.code)
            .bind(&c.client_id)
            .bind(&c.user_id)
            .bind(&c.redirect_uri)
            .bind(encode_json_array(&c.scopes))
            .bind(&c.code_challenge)
            .bind(&c.code_challenge_method)
            .bind(&c.nonce)
            .bind(&c.state)
            .bind(c.issued_at)
            .bind(c.expires_at)
            .bind(b(c.consumed))
            .execute(&self.pool)
            .await
            .context("auth.oidc_authorization_codes insert")?;
            Ok(())
        }

        async fn consume(&self, code: &str) -> Result<Option<AuthorizationCode>> {
            // SQLite has no RETURNING-after-UPDATE round-trip helper for
            // every backend version; do the load + conditional-update in
            // a transaction to preserve "consume returns row" semantics.
            let mut tx = self.pool.begin().await.context("begin consume tx")?;
            let row = sqlx::query(
                "SELECT * FROM auth.oidc_authorization_codes
                 WHERE code = ? AND consumed = 0",
            )
            .bind(code)
            .fetch_optional(&mut *tx)
            .await
            .context("auth.oidc_authorization_codes consume select")?;
            let Some(row) = row else {
                tx.rollback().await.ok();
                return Ok(None);
            };
            let result = sqlx::query(
                "UPDATE auth.oidc_authorization_codes SET consumed = 1 \
                 WHERE code = ? AND consumed = 0",
            )
            .bind(code)
            .execute(&mut *tx)
            .await
            .context("auth.oidc_authorization_codes consume update")?;
            if result.rows_affected() == 0 {
                tx.rollback().await.ok();
                return Ok(None);
            }
            tx.commit().await.context("commit consume tx")?;
            Ok(Some(map_code_row(row)))
        }
    }

    fn map_refresh_row(row: sqlx::sqlite::SqliteRow) -> RefreshToken {
        RefreshToken {
            token_hash: row.get("token_hash"),
            client_id: row.get("client_id"),
            user_id: row.get("user_id"),
            scopes: parse_json_array(&row.get::<String, _>("scopes")),
            issued_at: row.get("issued_at"),
            expires_at: row.get("expires_at"),
            revoked: ub(row.get("revoked")),
        }
    }

    #[derive(Clone)]
    pub struct SqliteOidcRefreshStore {
        pool: SqlitePool,
    }
    impl SqliteOidcRefreshStore {
        pub fn new(pool: SqlitePool) -> Self {
            Self { pool }
        }
        pub fn into_dyn(self) -> Arc<dyn OidcRefreshStore> {
            Arc::new(self)
        }
    }

    #[async_trait]
    impl OidcRefreshStore for SqliteOidcRefreshStore {
        async fn create(&self, t: &RefreshToken) -> Result<()> {
            sqlx::query(
                "INSERT INTO auth.oidc_refresh_tokens
                    (token_hash, client_id, user_id, scopes,
                     issued_at, expires_at, revoked)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&t.token_hash)
            .bind(&t.client_id)
            .bind(&t.user_id)
            .bind(encode_json_array(&t.scopes))
            .bind(t.issued_at)
            .bind(t.expires_at)
            .bind(b(t.revoked))
            .execute(&self.pool)
            .await
            .context("auth.oidc_refresh_tokens insert")?;
            Ok(())
        }

        async fn get(&self, token_hash: &str) -> Result<Option<RefreshToken>> {
            let row = sqlx::query(
                "SELECT * FROM auth.oidc_refresh_tokens WHERE token_hash = ?",
            )
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .context("auth.oidc_refresh_tokens get")?;
            Ok(row.map(map_refresh_row))
        }

        async fn revoke(&self, token_hash: &str) -> Result<bool> {
            let r = sqlx::query(
                "UPDATE auth.oidc_refresh_tokens SET revoked = 1 WHERE token_hash = ?",
            )
            .bind(token_hash)
            .execute(&self.pool)
            .await
            .context("auth.oidc_refresh_tokens revoke")?;
            Ok(r.rows_affected() > 0)
        }

        async fn revoke_for_user(&self, user_id: &str) -> Result<u64> {
            let r = sqlx::query(
                "UPDATE auth.oidc_refresh_tokens SET revoked = 1 WHERE user_id = ?",
            )
            .bind(user_id)
            .execute(&self.pool)
            .await
            .context("auth.oidc_refresh_tokens revoke_for_user")?;
            Ok(r.rows_affected())
        }
    }

    fn map_session_row(row: sqlx::sqlite::SqliteRow) -> OidcSession {
        OidcSession {
            sid: row.get("sid"),
            user_id: row.get("user_id"),
            client_id: row.get("client_id"),
            assay_session_id: row.get("assay_session_id"),
            issued_at: row.get("issued_at"),
            backchannel_logout_uri: row.get("backchannel_logout_uri"),
        }
    }

    #[derive(Clone)]
    pub struct SqliteOidcSessionStore {
        pool: SqlitePool,
    }
    impl SqliteOidcSessionStore {
        pub fn new(pool: SqlitePool) -> Self {
            Self { pool }
        }
        pub fn into_dyn(self) -> Arc<dyn OidcSessionStore> {
            Arc::new(self)
        }
    }

    #[async_trait]
    impl OidcSessionStore for SqliteOidcSessionStore {
        async fn create(&self, s: &OidcSession) -> Result<()> {
            sqlx::query(
                "INSERT INTO auth.oidc_sessions
                    (sid, user_id, client_id, assay_session_id, issued_at, backchannel_logout_uri)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&s.sid)
            .bind(&s.user_id)
            .bind(&s.client_id)
            .bind(&s.assay_session_id)
            .bind(s.issued_at)
            .bind(&s.backchannel_logout_uri)
            .execute(&self.pool)
            .await
            .context("auth.oidc_sessions insert")?;
            Ok(())
        }

        async fn get(&self, sid: &str) -> Result<Option<OidcSession>> {
            let row = sqlx::query("SELECT * FROM auth.oidc_sessions WHERE sid = ?")
                .bind(sid)
                .fetch_optional(&self.pool)
                .await
                .context("auth.oidc_sessions get")?;
            Ok(row.map(map_session_row))
        }

        async fn list_by_assay_session(&self, assay_session_id: &str) -> Result<Vec<OidcSession>> {
            let rows = sqlx::query(
                "SELECT * FROM auth.oidc_sessions WHERE assay_session_id = ?",
            )
            .bind(assay_session_id)
            .fetch_all(&self.pool)
            .await
            .context("auth.oidc_sessions list_by_assay_session")?;
            Ok(rows.into_iter().map(map_session_row).collect())
        }

        async fn delete(&self, sid: &str) -> Result<bool> {
            let r = sqlx::query("DELETE FROM auth.oidc_sessions WHERE sid = ?")
                .bind(sid)
                .execute(&self.pool)
                .await
                .context("auth.oidc_sessions delete")?;
            Ok(r.rows_affected() > 0)
        }

        async fn delete_by_assay_session(&self, assay_session_id: &str) -> Result<u64> {
            let r = sqlx::query(
                "DELETE FROM auth.oidc_sessions WHERE assay_session_id = ?",
            )
            .bind(assay_session_id)
            .execute(&self.pool)
            .await
            .context("auth.oidc_sessions delete_by_assay_session")?;
            Ok(r.rows_affected())
        }
    }

    fn map_consent_row(row: sqlx::sqlite::SqliteRow) -> ConsentGrant {
        ConsentGrant {
            user_id: row.get("user_id"),
            client_id: row.get("client_id"),
            scopes: parse_json_array(&row.get::<String, _>("scopes")),
            granted_at: row.get("granted_at"),
        }
    }

    #[derive(Clone)]
    pub struct SqliteOidcConsentStore {
        pool: SqlitePool,
    }
    impl SqliteOidcConsentStore {
        pub fn new(pool: SqlitePool) -> Self {
            Self { pool }
        }
        pub fn into_dyn(self) -> Arc<dyn OidcConsentStore> {
            Arc::new(self)
        }
    }

    #[async_trait]
    impl OidcConsentStore for SqliteOidcConsentStore {
        async fn upsert(&self, g: &ConsentGrant) -> Result<()> {
            sqlx::query(
                "INSERT INTO auth.oidc_consents (user_id, client_id, scopes, granted_at)
                 VALUES (?, ?, ?, ?)
                 ON CONFLICT (user_id, client_id) DO UPDATE
                     SET scopes = excluded.scopes,
                         granted_at = excluded.granted_at",
            )
            .bind(&g.user_id)
            .bind(&g.client_id)
            .bind(encode_json_array(&g.scopes))
            .bind(g.granted_at)
            .execute(&self.pool)
            .await
            .context("auth.oidc_consents upsert")?;
            Ok(())
        }

        async fn get(&self, user_id: &str, client_id: &str) -> Result<Option<ConsentGrant>> {
            let row = sqlx::query(
                "SELECT * FROM auth.oidc_consents WHERE user_id = ? AND client_id = ?",
            )
            .bind(user_id)
            .bind(client_id)
            .fetch_optional(&self.pool)
            .await
            .context("auth.oidc_consents get")?;
            Ok(row.map(map_consent_row))
        }

        async fn delete(&self, user_id: &str, client_id: &str) -> Result<bool> {
            let r = sqlx::query(
                "DELETE FROM auth.oidc_consents WHERE user_id = ? AND client_id = ?",
            )
            .bind(user_id)
            .bind(client_id)
            .execute(&self.pool)
            .await
            .context("auth.oidc_consents delete")?;
            Ok(r.rows_affected() > 0)
        }
    }

    fn map_upstream_state_row(row: sqlx::sqlite::SqliteRow) -> UpstreamLoginState {
        UpstreamLoginState {
            state: row.get("state"),
            provider_slug: row.get("provider_slug"),
            nonce: row.get("nonce"),
            pkce_verifier: row.get("pkce_verifier"),
            return_to: row.get("return_to"),
            created_at: row.get("created_at"),
            expires_at: row.get("expires_at"),
        }
    }

    #[derive(Clone)]
    pub struct SqliteOidcUpstreamStateStore {
        pool: SqlitePool,
    }
    impl SqliteOidcUpstreamStateStore {
        pub fn new(pool: SqlitePool) -> Self {
            Self { pool }
        }
        pub fn into_dyn(self) -> Arc<dyn OidcUpstreamStateStore> {
            Arc::new(self)
        }
    }

    #[async_trait]
    impl OidcUpstreamStateStore for SqliteOidcUpstreamStateStore {
        async fn create(&self, s: &UpstreamLoginState) -> Result<()> {
            sqlx::query(
                "INSERT INTO auth.oidc_upstream_states
                    (state, provider_slug, nonce, pkce_verifier, return_to,
                     created_at, expires_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&s.state)
            .bind(&s.provider_slug)
            .bind(&s.nonce)
            .bind(&s.pkce_verifier)
            .bind(&s.return_to)
            .bind(s.created_at)
            .bind(s.expires_at)
            .execute(&self.pool)
            .await
            .context("auth.oidc_upstream_states insert")?;
            Ok(())
        }

        async fn take(&self, state: &str) -> Result<Option<UpstreamLoginState>> {
            // SQLite likewise lacks RETURNING in some sqlx mappings; do a
            // load + delete in a tx for parity with the PG behaviour.
            let mut tx = self.pool.begin().await.context("begin take tx")?;
            let row = sqlx::query(
                "SELECT * FROM auth.oidc_upstream_states WHERE state = ?",
            )
            .bind(state)
            .fetch_optional(&mut *tx)
            .await
            .context("auth.oidc_upstream_states take select")?;
            let Some(row) = row else {
                tx.rollback().await.ok();
                return Ok(None);
            };
            sqlx::query("DELETE FROM auth.oidc_upstream_states WHERE state = ?")
                .bind(state)
                .execute(&mut *tx)
                .await
                .context("auth.oidc_upstream_states take delete")?;
            tx.commit().await.context("commit take tx")?;
            Ok(Some(map_upstream_state_row(row)))
        }
    }
}

#[cfg(feature = "backend-sqlite")]
pub use sqlite_impl::{
    SqliteOidcClientStore, SqliteOidcCodeStore, SqliteOidcConsentStore, SqliteOidcRefreshStore,
    SqliteOidcSessionStore, SqliteOidcUpstreamStateStore, SqliteOidcUpstreamStore,
};
