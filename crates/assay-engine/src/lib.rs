//! Assay engine — workflow + dashboard as a crate or standalone binary.
//!
//! Phase 3 composes `assay-workflow` and `assay-dashboard` behind one HTTP
//! port. Auth (`assay-auth`) is reserved for Phase 8 — feature-gated
//! behind `auth` so the crate compiles identically with or without it
//! until the auth modules land.
//!
//! See plan 12 § Architecture principle 1 for the composition model and
//! § Architecture principle 8 for the runtime/engine split.

use std::sync::Arc;

use assay_dashboard::{DashboardCtx, WhitelabelConfig};
use assay_workflow::{PostgresStore, SqliteStore, WorkflowStore};

pub mod config;
pub mod server;
pub mod state;

pub use assay_domain as core;
pub use assay_dashboard as dashboard;
pub use assay_workflow as workflow;

#[cfg(feature = "auth")]
pub use assay_auth as auth;

pub use config::{BackendConfig, DashboardConfig, EngineConfig, ServerConfig};
pub use state::EngineState;

/// Top-level entrypoint: pick the backend from config, build state, serve.
pub async fn run(cfg: EngineConfig) -> anyhow::Result<()> {
    match cfg.backend.clone() {
        BackendConfig::Postgres { url } => {
            let store = PostgresStore::new(&url)
                .await
                .map_err(|e| anyhow::anyhow!("connect postgres: {e}"))?;
            run_with_store(cfg, store).await
        }
        BackendConfig::Sqlite { path } => {
            let url = if path == ":memory:" {
                "sqlite::memory:".to_string()
            } else {
                format!("sqlite://{}?mode=rwc", path)
            };
            let store = SqliteStore::new(&url)
                .await
                .map_err(|e| anyhow::anyhow!("connect sqlite: {e}"))?;
            run_with_store(cfg, store).await
        }
    }
}

async fn run_with_store<S: WorkflowStore + 'static>(
    cfg: EngineConfig,
    store: S,
) -> anyhow::Result<()> {
    let workflow_ctx = server::build_workflow_ctx(store);
    let whitelabel = Arc::new(WhitelabelConfig::from_env());
    let asset_version = env!("CARGO_PKG_VERSION").to_string();
    let dashboard_ctx = Arc::new(DashboardCtx::new(whitelabel, asset_version));
    let state = EngineState {
        workflow: workflow_ctx,
        dashboard: dashboard_ctx,
    };
    server::serve(&cfg.server.bind_addr, state).await
}
