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

pub mod config;
pub mod embedded;
pub mod engine_api;
pub mod init;
pub mod server;
pub mod state;

pub use assay_auth as auth;
pub use assay_dashboard as dashboard;
pub use assay_domain as core;
pub use assay_workflow as workflow;

pub use config::{
    AuthConfig, AuthOidcProviderConfig, AuthPasskeyConfig, AuthSessionConfig, BackendConfig,
    DashboardConfig, EngineConfig, ServerConfig,
};
pub use state::{AdminApiKeys, EngineState};

/// Top-level entrypoint for the standalone `assay-engine` binary.
/// Picks the backend from config, composes engine via
/// [`embedded::build`], and serves forever on `cfg.server.bind_addr`.
///
/// For embedded use (composing engine into a parent binary's
/// router), call [`embedded::build`] directly.
pub async fn run(cfg: EngineConfig) -> anyhow::Result<()> {
    let bind_addr = cfg.server.bind_addr.clone();
    let engine = embedded::build(cfg).await?;
    server::bind_and_serve(&bind_addr, engine.router).await
}

/// Build the vault context iff the runtime `engine.modules.vault.enabled`
/// row is TRUE. Loads the master KEK from `vault.kek_metadata` (or seeds
/// a fresh one on first boot) and composes the per-feature stores
/// against the same pool the rest of the engine uses.
#[cfg(all(feature = "vault", feature = "backend-postgres"))]
async fn build_vault_ctx_pg(
    modules: &[String],
    pool: &sqlx::PgPool,
) -> anyhow::Result<Option<assay_vault::VaultCtx>> {
    if !modules.iter().any(|m| m == "vault") {
        return Ok(None);
    }
    let kek = assay_vault::crypto::kek_store::load_or_init_postgres(pool)
        .await
        .map_err(|e| anyhow::anyhow!("vault KEK bootstrap (pg): {e}"))?;
    // The `vault` umbrella feature on assay-vault implies vault-kv +
    // vault-transit, so the with_* methods are unconditionally
    // available here.
    let mut ctx = assay_vault::VaultCtx::new()
        .with_kek(kek)
        .with_kv(assay_vault::store::postgres::PgKvStore::new(pool.clone()))
        .with_transit(assay_vault::store::postgres::PgTransitStore::new(
            pool.clone(),
        ));
    #[cfg(feature = "vault-sealing-shamir")]
    {
        ctx = ctx.with_seal_store(assay_vault::store::postgres::PgSealStore::new(pool.clone()));
    }
    #[cfg(feature = "vault-collections")]
    {
        ctx = ctx
            .with_personal_vaults(assay_vault::store::postgres::PgPersonalVaultStore::new(
                pool.clone(),
            ))
            .with_collections(assay_vault::store::postgres::PgCollectionStore::new(
                pool.clone(),
            ))
            .with_items(assay_vault::store::postgres::PgItemStore::new(pool.clone()))
            .with_folders(assay_vault::store::postgres::PgFolderStore::new(
                pool.clone(),
            ));
        // Plan §S4 — seed default Zanzibar namespaces (vault,
        // collection, kv_path, team, family, org). Idempotent.
    }
    #[cfg(feature = "vault-share")]
    {
        let kp = assay_vault::store::postgres::load_or_init_biscuit_root_postgres(pool)
            .await
            .map_err(|e| anyhow::anyhow!("vault biscuit root bootstrap (pg): {e}"))?;
        let revs = std::sync::Arc::new(assay_vault::store::postgres::PgRevocationStore::new(
            pool.clone(),
        ));
        let svc = assay_vault::share::ShareService::new(kp, revs);
        ctx = ctx.with_share(svc);
    }
    #[cfg(feature = "vault-dynamic-postgres")]
    {
        let leases = std::sync::Arc::new(assay_vault::store::postgres::PgLeaseStore::new(
            pool.clone(),
        ));
        let registry = assay_vault::dynamic::DynamicCredsRegistry::new();
        // Phase 5 default-config: registry is empty until an operator
        // configures providers via /dynamic/* admin routes (or in
        // future, engine.toml). The dispatcher returns NotFound for
        // unknown providers, which surfaces as 404 to the caller.
        let svc = assay_vault::dynamic::DynamicCredsService::new(registry, leases);
        ctx = ctx.with_dynamic(svc);
    }
    Ok(Some(ctx))
}

/// SQLite mirror of [`build_vault_ctx_pg`].
#[cfg(all(feature = "vault", feature = "backend-sqlite"))]
async fn build_vault_ctx_sqlite(
    modules: &[String],
    pool: &sqlx::SqlitePool,
) -> anyhow::Result<Option<assay_vault::VaultCtx>> {
    if !modules.iter().any(|m| m == "vault") {
        return Ok(None);
    }
    let kek = assay_vault::crypto::kek_store::load_or_init_sqlite(pool)
        .await
        .map_err(|e| anyhow::anyhow!("vault KEK bootstrap (sqlite): {e}"))?;
    let mut ctx = assay_vault::VaultCtx::new()
        .with_kek(kek)
        .with_kv(assay_vault::store::sqlite::SqliteKvStore::new(pool.clone()))
        .with_transit(assay_vault::store::sqlite::SqliteTransitStore::new(
            pool.clone(),
        ));
    #[cfg(feature = "vault-sealing-shamir")]
    {
        ctx = ctx.with_seal_store(assay_vault::store::sqlite::SqliteSealStore::new(
            pool.clone(),
        ));
    }
    #[cfg(feature = "vault-collections")]
    {
        ctx = ctx
            .with_personal_vaults(assay_vault::store::sqlite::SqlitePersonalVaultStore::new(
                pool.clone(),
            ))
            .with_collections(assay_vault::store::sqlite::SqliteCollectionStore::new(
                pool.clone(),
            ))
            .with_items(assay_vault::store::sqlite::SqliteItemStore::new(
                pool.clone(),
            ))
            .with_folders(assay_vault::store::sqlite::SqliteFolderStore::new(
                pool.clone(),
            ));
    }
    #[cfg(feature = "vault-share")]
    {
        let kp = assay_vault::store::sqlite::load_or_init_biscuit_root_sqlite(pool)
            .await
            .map_err(|e| anyhow::anyhow!("vault biscuit root bootstrap (sqlite): {e}"))?;
        let revs = std::sync::Arc::new(assay_vault::store::sqlite::SqliteRevocationStore::new(
            pool.clone(),
        ));
        let svc = assay_vault::share::ShareService::new(kp, revs);
        ctx = ctx.with_share(svc);
    }
    #[cfg(feature = "vault-dynamic-postgres")]
    {
        let leases = std::sync::Arc::new(assay_vault::store::sqlite::SqliteLeaseStore::new(
            pool.clone(),
        ));
        let registry = assay_vault::dynamic::DynamicCredsRegistry::new();
        let svc = assay_vault::dynamic::DynamicCredsService::new(registry, leases);
        ctx = ctx.with_dynamic(svc);
    }
    Ok(Some(ctx))
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

        ctx = ctx.with_external_issuers(discover_external_issuers(cfg).await?);
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
        let zanzibar: Arc<dyn assay_auth::zanzibar::ZanzibarStore> = Arc::new(
            assay_auth::zanzibar::PostgresZanzibarStore::new(pool.clone()),
        );
        ctx = ctx.with_zanzibar(zanzibar);
    }

    #[cfg(feature = "auth-oidc-provider")]
    if cfg.auth.oidc_provider.enabled {
        let issuer = oidc_issuer(cfg);
        let public_url = oidc_public_url(cfg)?;
        let provider = assay_auth::oidc_provider::OidcProviderConfig::new(
            issuer,
            public_url,
            assay_auth::oidc_provider::PostgresOidcClientStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::PostgresOidcUpstreamStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::PostgresOidcCodeStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::PostgresOidcRefreshStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::PostgresOidcSessionStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::PostgresOidcConsentStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::PostgresOidcUpstreamStateStore::new(pool.clone()).into_dyn(),
        )
        .with_jwks_source(assay_auth::oidc_provider::JwksSource::Postgres(
            pool.clone(),
        ))
        .with_auto_provision(cfg.auth.oidc_provider.auto_provision);
        ctx = ctx.with_oidc_provider(provider);

        if let (Some(registry), Some(provider)) = (&ctx.oidc, &ctx.oidc_provider) {
            match provider.upstream.list().await {
                Ok(rows) => {
                    for row in rows {
                        assay_auth::oidc_provider::sync_upstream_to_registry(
                            registry,
                            &row,
                            &provider.public_url,
                        )
                        .await;
                    }
                }
                Err(e) => {
                    tracing::warn!("failed to list upstream providers at boot: {e}");
                }
            }
        }
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

        ctx = ctx.with_external_issuers(discover_external_issuers(cfg).await?);
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
        let public_url = oidc_public_url(cfg)?;
        let provider = assay_auth::oidc_provider::OidcProviderConfig::new(
            issuer,
            public_url,
            assay_auth::oidc_provider::SqliteOidcClientStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::SqliteOidcUpstreamStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::SqliteOidcCodeStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::SqliteOidcRefreshStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::SqliteOidcSessionStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::SqliteOidcConsentStore::new(pool.clone()).into_dyn(),
            assay_auth::oidc_provider::SqliteOidcUpstreamStateStore::new(pool.clone()).into_dyn(),
        )
        .with_jwks_source(assay_auth::oidc_provider::JwksSource::Sqlite(pool.clone()))
        .with_auto_provision(cfg.auth.oidc_provider.auto_provision);
        ctx = ctx.with_oidc_provider(provider);

        if let (Some(registry), Some(provider)) = (&ctx.oidc, &ctx.oidc_provider) {
            match provider.upstream.list().await {
                Ok(rows) => {
                    for row in rows {
                        assay_auth::oidc_provider::sync_upstream_to_registry(
                            registry,
                            &row,
                            &provider.public_url,
                        )
                        .await;
                    }
                }
                Err(e) => {
                    tracing::warn!("failed to list upstream providers at boot: {e}");
                }
            }
        }
    }

    Ok(ctx)
}

/// Discover each configured external OIDC issuer once at boot and
/// hand back ready-to-use verifiers. Each verifier owns a background
/// task that refreshes its JWKS on the configured interval.
///
/// Errors here are fatal — if Hydra (or whichever IdP) is unreachable
/// at boot the engine shouldn't pretend it can validate tokens. The
/// alternative (silently degrading to "no external issuer trusted")
/// would surface as 401s and look like a session bug.
#[cfg(feature = "auth-jwt")]
async fn discover_external_issuers(
    cfg: &EngineConfig,
) -> anyhow::Result<Vec<assay_auth::external_jwt::ExternalJwtIssuer>> {
    let entries = cfg.auth.external_issuers();
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let verifier = assay_auth::external_jwt::ExternalJwtIssuer::discover(
            entry.issuer_url.clone(),
            entry.audience.clone(),
            entry.jwks_refresh_secs,
        )
        .await
        .map_err(|e| anyhow::anyhow!("discover external issuer `{}`: {e}", entry.issuer_url))?;
        tracing::info!(
            target: "assay-engine",
            issuer = %entry.issuer_url,
            audience = ?entry.audience,
            "trusted external OIDC issuer for JWT pass-through"
        );
        out.push(verifier);
    }
    Ok(out)
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

/// Parse `server.public_url` as a `url::Url`. Used by passkey RP setup
/// (which wants the bare origin) — not by the OIDC provider, which
/// needs the issuer URL (with `/auth`); see [`oidc_public_url`].
#[cfg(feature = "auth-passkey")]
fn parse_public_url(cfg: &EngineConfig) -> anyhow::Result<url::Url> {
    url::Url::parse(&cfg.server.public_url)
        .map_err(|e| anyhow::anyhow!("server.public_url {:?}: {e}", cfg.server.public_url))
}

/// Base URL the OIDC provider exposes its endpoints at — same as
/// [`oidc_issuer`] (which already accounts for the `/auth` mount
/// prefix), parsed as a `url::Url`. Passed into `OidcProviderConfig`
/// so `upstream_callback_url(...)` produces an absolute URI that
/// matches the actual handler path.
fn oidc_public_url(cfg: &EngineConfig) -> anyhow::Result<url::Url> {
    let issuer = oidc_issuer(cfg);
    url::Url::parse(&issuer).map_err(|e| anyhow::anyhow!("oidc issuer {issuer:?}: {e}"))
}

/// Build a passkey manager from `auth.passkey` config. Returns `None`
/// when the public_url isn't parseable as a URL with a host (passkeys
/// require an origin) — we log + skip rather than fail boot.
#[cfg(feature = "auth-passkey")]
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

// `run_with_store` (the previous private composition helper) is gone.
// Its body lives in `embedded::compose` (this module's `embedded` sibling),
// minus the final `server::serve` call. `pub async fn run` above
// composes the engine via `embedded::build` and then binds + serves
// the resulting `axum::Router` via `server::bind_and_serve`.
