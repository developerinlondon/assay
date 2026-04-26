//! Engine configuration loaded from TOML.
//!
//! Phase 8 wires in `AuthConfig` so the engine binary can compose an
//! `assay_auth::AuthCtx` per-deployment (issuer, OIDC provider toggle,
//! session/cookie shape). When `auth` isn't compiled in (Cargo feature
//! off) the auth section is parsed but never read — keeping the TOML
//! shape stable across feature configurations.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EngineConfig {
    pub server: ServerConfig,
    pub backend: BackendConfig,
    #[serde(default)]
    pub workflow: WorkflowConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub dashboard: DashboardConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    /// TTL in seconds for the engine_events outbox. Rows older than this
    /// are pruned hourly by the cleanup loop. Default 3 days.
    #[serde(default = "default_engine_events_ttl_secs")]
    pub engine_events_ttl_secs: u64,
    /// Modules to flip from `enabled = FALSE` to `enabled = TRUE` on
    /// first boot when they're compiled in. Empty by default — operators
    /// of existing v0.1.2 deployments shouldn't get unexpected auth
    /// migrations on upgrade. Local-dev convenience: set to
    /// `["auth"]` in `engine.local.toml` to flip auth on without an
    /// extra step.
    #[serde(default)]
    pub auto_enable_modules: Vec<String>,
}

fn default_engine_events_ttl_secs() -> u64 {
    3 * 86_400
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,
    /// Operator-supplied canonical URL the engine is reached at — used
    /// as the OIDC `iss` claim, biscuit token issuer, passkey origin,
    /// and the base for federation callbacks. Defaults to the bind addr
    /// over plain HTTP for local dev convenience; production deployments
    /// MUST override this with the public HTTPS URL.
    #[serde(default = "default_public_url")]
    pub public_url: String,
}

fn default_bind_addr() -> String {
    "0.0.0.0:3000".to_string()
}

fn default_public_url() -> String {
    "http://localhost:3000".to_string()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BackendConfig {
    Postgres {
        /// Postgres connection URL, e.g. `postgres://user:pass@host:5432/db`.
        /// PostgreSQL 18 is the minimum supported version (see plan 12 Principle 7).
        url: String,
    },
    Sqlite {
        /// Directory holding the per-module SQLite files
        /// (`<data_dir>/engine.db`, `<data_dir>/workflow.db`, …). Created
        /// on startup if missing. Defaults to `./data`. Use `:memory:`
        /// in `path` (legacy) or set `data_dir = ":memory:"` to keep the
        /// engine purely in-memory for tests.
        #[serde(default = "default_data_dir")]
        data_dir: String,
        /// Legacy single-file SQLite path. Deprecated in v0.1.2 — when
        /// set, the engine logs a deprecation notice and treats it as
        /// `data_dir = parent(path)` so existing configs keep working
        /// during the transition.
        #[serde(default)]
        path: Option<String>,
    },
}

fn default_data_dir() -> String {
    "./data".to_string()
}

impl BackendConfig {
    /// Resolve the effective data directory for SQLite. PG returns `None`.
    pub fn sqlite_data_dir(&self) -> Option<String> {
        match self {
            Self::Sqlite { data_dir, path } => {
                // Legacy `path` wins for backwards compat — treat the
                // parent dir as the new data_dir so existing v0.1.1
                // configs migrate without surprise.
                if let Some(p) = path {
                    let parent = std::path::Path::new(p)
                        .parent()
                        .map(|p| p.display().to_string())
                        .filter(|s| !s.is_empty());
                    Some(parent.unwrap_or_else(|| data_dir.clone()))
                } else {
                    Some(data_dir.clone())
                }
            }
            Self::Postgres { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WorkflowConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Auth-module deployment shape. Read by the engine binary when the
/// `auth` Cargo feature is compiled in AND `engine.modules.auth.enabled`
/// is TRUE; otherwise the defaults are harmless.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AuthConfig {
    /// JWT issuer + OIDC `iss` claim. Defaults to
    /// `<server.public_url>/auth` when unset, which matches the route
    /// mount point.
    pub issuer: Option<String>,
    /// JWT audience list — also used by the OIDC provider when minting
    /// access_tokens for resource servers. Defaults to `[issuer]`.
    #[serde(default)]
    pub audience: Vec<String>,
    #[serde(default)]
    pub session: AuthSessionConfig,
    #[serde(default)]
    pub passkey: AuthPasskeyConfig,
    #[serde(default)]
    pub oidc_provider: AuthOidcProviderConfig,
    /// Admin API keys — comma-separated bearer tokens that grant access
    /// to `/admin/*` routes. Operators rotate these via the engine
    /// config. Per-token, no expiry; for fancier admin auth (Zanzibar
    /// roles, session-based admin) see plan 12c § 6.7. Empty list locks
    /// admin routes entirely (404 → 401).
    #[serde(default)]
    pub admin_api_keys: Vec<String>,
}

/// Session module knobs.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AuthSessionConfig {
    /// Default session lifetime in seconds. `None` ⇒ uses the
    /// `assay_auth::session::DEFAULT_SESSION_DURATION` (30 days).
    pub ttl_seconds: Option<u64>,
}

/// WebAuthn / passkey module knobs.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AuthPasskeyConfig {
    /// Relying-party id — the host (no scheme/port) the browser will
    /// scope passkeys to. Defaults to the host of `server.public_url`.
    pub rp_id: Option<String>,
    /// Human-readable label browsers show. Defaults to `"Assay"`.
    pub rp_name: Option<String>,
}

/// OIDC provider knobs.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AuthOidcProviderConfig {
    /// Whether the OIDC provider routes (/authorize /token /userinfo …)
    /// are mounted. Defaults to `true` when the Cargo feature is on.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Override the issuer URL used by the OIDC provider. Defaults to
    /// the parent [`AuthConfig::issuer`] when unset.
    pub issuer_override: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DashboardConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_log_format")]
    pub format: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_log_format() -> String {
    "pretty".to_string()
}

impl EngineConfig {
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("read config {}: {e}", path.display()))?;
        let cfg: Self = toml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("parse config {}: {e}", path.display()))?;
        Ok(cfg)
    }
}
