//! `assay-engine` — one static binary that replaces a Temporal +
//! Kratos + Hydra + Keto stack.
//!
//! `v0.2.0` is the umbrella release that turns the engine into a full
//! IdP + workflow runtime: the previously-empty [`auth`] feature now
//! pulls [`assay_auth`] in, mounting OIDC client + provider, passkey,
//! Argon2 password, JWT + JWKS rotation, biscuit capability tokens,
//! Zanzibar ReBAC, session + admin endpoints under `/auth`. The
//! dashboard panes that consume those routes (Users, Sessions, OIDC
//! clients, Upstream providers, Zanzibar, JWKS, Biscuit, Audit) light up
//! when the auth module is enabled in `engine.modules`.
//!
//! Composition is via [`axum::extract::FromRef`] over [`EngineState<S>`]
//! — workflow / auth / dashboard each contribute their own `Ctx` and
//! the parent state derives every sub-state extractor automatically. A
//! no-auth build (`--no-default-features --features
//! "backend-postgres,backend-sqlite"`) compiles identically to the
//! pre-v0.2.0 engine; an auth build composes the auth ctx if and only if
//! `engine.modules` shows `auth` enabled at boot.
//!
//! ## Module enablement model
//!
//! Three layers compose:
//!
//! 1. **Compile features (Cargo)** — decide whether the module's code
//!    is *linked* into the binary. `assay-engine`'s default compiles
//!    workflow + dashboard; opt into `auth` for the IdP.
//! 2. **`engine.modules` row (DB)** — decides whether the module is
//!    *active* at runtime. `name`, `enabled`, `version`, `config`. The
//!    boot path runs the module's migrations + mounts its routes + lets
//!    the dashboard render its panes only when `enabled = TRUE`.
//! 3. **`engine.toml` config** — decides how the active module is
//!    *configured* (issuer URL, session TTL, OIDC provider toggle,
//!    admin api-keys, …).
//!
//! See plan 12 § Architecture principle 1 (composition) and § principle
//! 8 (runtime/engine split). Migration notes for v0.1.x → v0.2.0 live
//! in `docs/migration-to-0.2.0.md`.

use std::sync::Arc;

use assay_dashboard::{DashboardCtx, WhitelabelConfig};
use assay_domain::events::EngineEventBus;
use assay_workflow::WorkflowStore;

pub mod config;
pub mod engine_api;
pub mod init;
pub mod server;
pub mod state;

pub use assay_auth as auth;
pub use assay_domain as core;
pub use assay_dashboard as dashboard;
pub use assay_workflow as workflow;

pub use config::{
    AuthConfig, AuthOidcProviderConfig, AuthPasskeyConfig, AuthSessionConfig, BackendConfig,
    DashboardConfig, EngineConfig, ServerConfig,
};
pub use state::{AdminApiKeys, EngineState};

/// Top-level entrypoint: pick the backend from config, build state, serve.
pub async fn run(cfg: EngineConfig) -> anyhow::Result<()> {
    let boot = init::EngineBoot::run(&cfg).await?;
    match boot {
        #[cfg(feature = "backend-postgres")]
        init::EngineBoot::Postgres(b) => {
            let store = assay_workflow::PostgresStore::from_pool(b.pool.clone())
                .await
                .map_err(|e| anyhow::anyhow!("workflow store (pg): {e}"))?;
            let auth_ctx = build_auth_ctx_pg(&cfg, &b.pool).await?;
            run_with_store(cfg, store, b.bus, b.modules, b.instance_id, Some(auth_ctx))
                .await
        }
        #[cfg(feature = "backend-sqlite")]
        init::EngineBoot::Sqlite(b) => {
            let store = assay_workflow::SqliteStore::from_attached_pool(b.pool.clone())
                .await
                .map_err(|e| anyhow::anyhow!("workflow store (sqlite): {e}"))?;
            let auth_ctx = build_auth_ctx_sqlite(&cfg, &b.pool).await?;
            run_with_store(cfg, store, b.bus, b.modules, b.instance_id, Some(auth_ctx))
                .await
        }
    }
}

#[cfg(feature = "backend-postgres")]
async fn build_auth_ctx_pg(
    cfg: &EngineConfig,
    pool: &sqlx::PgPool,
) -> anyhow::Result<assay_auth::AuthCtx> {
    use assay_auth::store::{PostgresSessionStore, PostgresUserStore};
    let users = PostgresUserStore::new(pool.clone()).into_dyn();
    let sessions = PostgresSessionStore::new(pool.clone()).into_dyn();
    let mut ctx = assay_auth::AuthCtx::new(users.clone(), sessions);

    let biscuit = assay_auth::biscuit::load_or_init_postgres(pool)
        .await
        .map_err(|e| anyhow::anyhow!("biscuit root key (pg): {e}"))?;
    ctx = ctx.with_biscuit(biscuit);

    #[cfg(feature = "auth-jwt")]
    {
        let issuer = effective_issuer(cfg);
        let audience = if cfg.auth.audience.is_empty() {
            vec![issuer.clone()]
        } else {
            cfg.auth.audience.clone()
        };
        let jwt = assay_auth::jwt::JwtConfig::new(issuer.clone(), audience);
        if let Err(e) = jwt.load_from_postgres(pool).await {
            tracing::warn!(?e, "no JWKS rows yet; rotating to seed first key");
            jwt.rotate_postgres(pool)
                .await
                .map_err(|e| anyhow::anyhow!("seed JWKS (pg): {e}"))?;
        }
        if jwt.active_kid().is_none() {
            jwt.rotate_postgres(pool)
                .await
                .map_err(|e| anyhow::anyhow!("seed JWKS (pg): {e}"))?;
        }
        ctx = ctx.with_jwt(jwt);
    }

    #[cfg(feature = "auth-oidc")]
    {
        ctx = ctx.with_oidc(assay_auth::oidc::OidcRegistry::new());
    }

    #[cfg(feature = "auth-passkey")]
    if let Some(passkey_mgr) = build_passkey_manager(cfg, users.clone()) {
        ctx = ctx.with_passkeys(passkey_mgr);
    }

    #[cfg(feature = "auth-zanzibar")]
    {
        let zanzibar: Arc<dyn assay_auth::zanzibar::ZanzibarStore> =
            Arc::new(assay_auth::zanzibar::PostgresZanzibarStore::new(pool.clone()));
        ctx = ctx.with_zanzibar(zanzibar);
    }

    #[cfg(feature = "auth-oidc-provider")]
    if cfg.auth.oidc_provider.enabled {
        let issuer = oidc_issuer(cfg);
        let public_url = parse_public_url(cfg)?;
        let provider = assay_auth::oidc_provider::OidcProviderConfig::new(
            issuer,
            public_url,
            assay_auth::oidc_provider::PostgresOidcClientStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::PostgresOidcUpstreamStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::PostgresOidcCodeStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::PostgresOidcRefreshStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::PostgresOidcSessionStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::PostgresOidcConsentStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::PostgresOidcUpstreamStateStore::new(pool.clone())
                .into_dyn(),
        )
        .with_jwks_source(assay_auth::oidc_provider::JwksSource::Postgres(pool.clone()));
        ctx = ctx.with_oidc_provider(provider);
    }

    Ok(ctx)
}

#[cfg(feature = "backend-sqlite")]
async fn build_auth_ctx_sqlite(
    cfg: &EngineConfig,
    pool: &sqlx::SqlitePool,
) -> anyhow::Result<assay_auth::AuthCtx> {
    use assay_auth::store::{SqliteSessionStore, SqliteUserStore};
    let users = SqliteUserStore::new(pool.clone()).into_dyn();
    let sessions = SqliteSessionStore::new(pool.clone()).into_dyn();
    let mut ctx = assay_auth::AuthCtx::new(users.clone(), sessions);

    let biscuit = assay_auth::biscuit::load_or_init_sqlite(pool)
        .await
        .map_err(|e| anyhow::anyhow!("biscuit root key (sqlite): {e}"))?;
    ctx = ctx.with_biscuit(biscuit);

    #[cfg(feature = "auth-jwt")]
    {
        let issuer = effective_issuer(cfg);
        let audience = if cfg.auth.audience.is_empty() {
            vec![issuer.clone()]
        } else {
            cfg.auth.audience.clone()
        };
        let jwt = assay_auth::jwt::JwtConfig::new(issuer.clone(), audience);
        if let Err(e) = jwt.load_from_sqlite(pool).await {
            tracing::warn!(?e, "no JWKS rows yet; rotating to seed first key");
            jwt.rotate_sqlite(pool)
                .await
                .map_err(|e| anyhow::anyhow!("seed JWKS (sqlite): {e}"))?;
        }
        if jwt.active_kid().is_none() {
            jwt.rotate_sqlite(pool)
                .await
                .map_err(|e| anyhow::anyhow!("seed JWKS (sqlite): {e}"))?;
        }
        ctx = ctx.with_jwt(jwt);
    }

    #[cfg(feature = "auth-oidc")]
    {
        ctx = ctx.with_oidc(assay_auth::oidc::OidcRegistry::new());
    }

    #[cfg(feature = "auth-passkey")]
    if let Some(passkey_mgr) = build_passkey_manager(cfg, users.clone()) {
        ctx = ctx.with_passkeys(passkey_mgr);
    }

    #[cfg(feature = "auth-zanzibar")]
    {
        let zanzibar: Arc<dyn assay_auth::zanzibar::ZanzibarStore> =
            Arc::new(assay_auth::zanzibar::SqliteZanzibarStore::new(pool.clone()));
        ctx = ctx.with_zanzibar(zanzibar);
    }

    #[cfg(feature = "auth-oidc-provider")]
    if cfg.auth.oidc_provider.enabled {
        let issuer = oidc_issuer(cfg);
        let public_url = parse_public_url(cfg)?;
        let provider = assay_auth::oidc_provider::OidcProviderConfig::new(
            issuer,
            public_url,
            assay_auth::oidc_provider::SqliteOidcClientStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::SqliteOidcUpstreamStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::SqliteOidcCodeStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::SqliteOidcRefreshStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::SqliteOidcSessionStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::SqliteOidcConsentStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::SqliteOidcUpstreamStateStore::new(pool.clone())
                .into_dyn(),
        )
        .with_jwks_source(assay_auth::oidc_provider::JwksSource::Sqlite(pool.clone()));
        ctx = ctx.with_oidc_provider(provider);
    }

    Ok(ctx)
}

/// Issuer for JWTs minted via the `auth-jwt` module. Defaults to
/// `<server.public_url>/auth` when unset, matching where the auth
/// router is mounted.
fn effective_issuer(cfg: &EngineConfig) -> String {
    if let Some(issuer) = &cfg.auth.issuer {
        return issuer.clone();
    }
    let base = cfg.server.public_url.trim_end_matches('/');
    format!("{base}/auth")
}

/// Issuer the OIDC provider advertises in its discovery doc + the `iss`
/// claim of every issued id_token. Defaults to the parent
/// [`effective_issuer`] when no override is set.
fn oidc_issuer(cfg: &EngineConfig) -> String {
    cfg.auth
        .oidc_provider
        .issuer_override
        .clone()
        .unwrap_or_else(|| effective_issuer(cfg))
}

/// Parse `server.public_url` as a `url::Url`. Used by the OIDC provider
/// to derive default redirect targets and by passkey RP setup.
fn parse_public_url(cfg: &EngineConfig) -> anyhow::Result<url::Url> {
    url::Url::parse(&cfg.server.public_url)
        .map_err(|e| anyhow::anyhow!("server.public_url {:?}: {e}", cfg.server.public_url))
}

/// Build a passkey manager from `auth.passkey` config. Returns `None`
/// when the public_url isn't parseable as a URL with a host (passkeys
/// require an origin) — we log + skip rather than fail boot.
fn build_passkey_manager(
    cfg: &EngineConfig,
    users: Arc<dyn assay_auth::store::UserStore>,
) -> Option<assay_auth::passkey::PasskeyManager> {
    let url = match parse_public_url(cfg) {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!(?e, "passkeys disabled — bad public_url");
            return None;
        }
    };
    let host = match url.host_str() {
        Some(h) => h.to_string(),
        None => {
            tracing::warn!("passkeys disabled — public_url has no host");
            return None;
        }
    };
    let pk_cfg = assay_auth::passkey::PasskeyConfig {
        rp_id: cfg.auth.passkey.rp_id.clone().unwrap_or(host),
        rp_name: cfg
            .auth
            .passkey
            .rp_name
            .clone()
            .unwrap_or_else(|| "Assay".to_string()),
        origin: url,
    };
    match assay_auth::passkey::PasskeyManager::new(pk_cfg, users) {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::warn!(?e, "passkeys disabled — manager construction failed");
            None
        }
    }
}

async fn run_with_store<S: WorkflowStore + Clone + 'static>(
    cfg: EngineConfig,
    store: S,
    bus: Arc<dyn EngineEventBus>,
    modules: Vec<String>,
    instance_id: uuid::Uuid,
    auth_ctx: Option<assay_auth::AuthCtx>,
) -> anyhow::Result<()> {
    // The engine refuses to start unless there's at least one operator
    // user (created via `bootstrap-admin`) or at least one entry in
    // `admin_api_keys` as a break-glass. Without either, every admin
    // request would 401 and the operator would be locked out — fail
    // fast with a helpful message instead.
    if let Some(auth) = auth_ctx.as_ref() {
        let user_count = auth
            .users
            .count_users(None)
            .await
            .map_err(|e| anyhow::anyhow!("count auth.users: {e}"))?;
        if user_count == 0 && cfg.auth.admin_api_keys.is_empty() {
            anyhow::bail!(
                "engine refuses to start: no operator users exist and \
                 `auth.admin_api_keys` is empty. Either run \
                 `assay-engine bootstrap-admin --email <e> --password <p>` \
                 to seed the first user, or add at least one entry to \
                 `auth.admin_api_keys` in engine.toml as a break-glass."
            );
        }
    }

    let workflow_ctx = server::build_workflow_ctx_with_bus(store, Arc::clone(&bus));

    // Hourly sweep of the engine_events outbox. Detached — the handle
    // lives for the process lifetime; there's nothing to await for
    // clean shutdown (prune is idempotent so a missed tick is fine).
    tokio::spawn(assay_workflow::events_cleanup::run_events_cleanup(
        Arc::clone(&bus),
        std::time::Duration::from_secs(3600),
        cfg.engine_events_ttl_secs,
    ));

    let whitelabel = Arc::new(WhitelabelConfig::from_env());
    let asset_version = env!("CARGO_PKG_VERSION").to_string();
    let dashboard_ctx = Arc::new(DashboardCtx::new(whitelabel, asset_version));
    let admin_api_keys = Arc::new(cfg.auth.admin_api_keys.clone());
    // Wall-clock seconds since epoch — uptime baseline for the
    // /api/v1/engine/core/info response. Captured here (just before serve)
    // so the value reflects the moment HTTP becomes ready, not the
    // earlier boot-sequence start.
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let bind_addr = cfg.server.bind_addr.clone();
    let engine_config = Arc::new(cfg);
    let state = EngineState {
        workflow: workflow_ctx,
        dashboard: dashboard_ctx,
        auth: auth_ctx,
        admin_api_keys,
        modules: Arc::new(modules),
        instance_id,
        engine_version: env!("CARGO_PKG_VERSION"),
        started_at,
        engine_config,
    };
    server::serve(&bind_addr, state).await
}
