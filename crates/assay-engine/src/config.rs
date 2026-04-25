//! Engine configuration loaded from TOML.
//!
//! Phase 3 scope: workflow + dashboard only (no auth). Auth-related
//! configuration (OIDC, sessions, JWT, etc.) slots into a sibling
//! `AuthConfig` field in Phase 8 — intentionally omitted here so
//! there's no half-wired auth surface in v0.13.0 pre-Phase-4 builds.

use serde::Deserialize;
use std::path::Path;

#[derive(Clone, Debug, Deserialize)]
pub struct EngineConfig {
    pub server: ServerConfig,
    pub backend: BackendConfig,
    #[serde(default)]
    pub workflow: WorkflowConfig,
    #[serde(default)]
    pub dashboard: DashboardConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    /// TTL in seconds for the engine_events outbox. Rows older than this
    /// are pruned hourly by the cleanup loop. Default 3 days.
    #[serde(default = "default_engine_events_ttl_secs")]
    pub engine_events_ttl_secs: u64,
}

fn default_engine_events_ttl_secs() -> u64 {
    3 * 86_400
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,
}

fn default_bind_addr() -> String {
    "0.0.0.0:3000".to_string()
}

#[derive(Clone, Debug, Deserialize)]
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

#[derive(Clone, Debug, Default, Deserialize)]
pub struct WorkflowConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct DashboardConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize)]
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
