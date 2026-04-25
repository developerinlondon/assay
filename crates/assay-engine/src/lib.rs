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
use assay_domain::events::EngineEventBus;
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
            // Bring the engine-core schema (engine.modules / .audit /
            // .instances / .migrations) up alongside the workflow tables.
            // Idempotent — safe to run on every boot.
            let engine_schema =
                assay_domain::engine::PgEngineSchema::new(store.pool().clone());
            engine_schema
                .migrate()
                .await
                .map_err(|e| anyhow::anyhow!("engine schema migrate (pg): {e}"))?;
            let bus: Arc<dyn EngineEventBus> = Arc::new(
                assay_domain::events::PgEngineEventBus::new(store.pool().clone(), &url)
                    .await
                    .map_err(|e| anyhow::anyhow!("engine-events bus (pg): {e}"))?,
            );
            run_with_store(cfg, store, bus).await
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
            // Phase 1: engine-core tables live in the main DB until
            // Phase 3 wires ATTACH. The schema layer reads `schema =
            // "main"` to address them.
            let engine_schema =
                assay_domain::engine::SqliteEngineSchema::new_in_main(store.pool().clone());
            engine_schema
                .migrate()
                .await
                .map_err(|e| anyhow::anyhow!("engine schema migrate (sqlite): {e}"))?;
            let bus: Arc<dyn EngineEventBus> = Arc::new(
                assay_domain::events::SqliteEngineEventBus::new(store.pool().clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("engine-events bus (sqlite): {e}"))?,
            );
            run_with_store(cfg, store, bus).await
        }
    }
}

async fn run_with_store<S: WorkflowStore + 'static>(
    cfg: EngineConfig,
    store: S,
    bus: Arc<dyn EngineEventBus>,
) -> anyhow::Result<()> {
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
    let state = EngineState {
        workflow: workflow_ctx,
        dashboard: dashboard_ctx,
    };
    server::serve(&cfg.server.bind_addr, state).await
}
