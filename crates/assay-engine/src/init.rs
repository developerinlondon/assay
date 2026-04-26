//! Engine boot sequence (v0.1.2 — schema/ATTACH layout).
//!
//! Implements the 8-step boot sequence from plan 14:
//!
//! 1. Open engine storage (PG: connect; SQLite: create data_dir + open
//!    a router connection that ATTACHes one file per module)
//! 2. Apply engine schema migrations (creates `engine.modules`,
//!    `engine.audit`, `engine.instances`, `engine.migrations`)
//! 3. Read `engine.modules` — on first boot seed it from the running
//!    build's compile-time modules; on subsequent boots just SELECT
//!    enabled modules
//! 4. For each enabled module: PG `CREATE SCHEMA IF NOT EXISTS <m>`
//!    or SQLite ensure-attached, then run module migrations
//! 5. Wire trait routing — handled by callers (engine binary builds the
//!    `WorkflowStore` against the prepared pool)
//! 6. Engine-level multi-node coordination:
//!    - PG: pg_try_advisory_lock(1) for leader election (existing path)
//!    - SQLite: engine.lock single-row exclusive (existing path)
//!    - Insert into `engine.instances` on startup, refresh on timer,
//!      DELETE on graceful shutdown
//! 7. Mount HTTP routers from each enabled module (caller wires them)
//! 8. Start scheduler, workers, etc. (caller wires them)
//!
//! [`EngineBoot`] returns the prepared pool(s), the engine-events bus,
//! the instance id, and the list of enabled modules — everything callers
//! need to compose `WorkflowStore`, `WorkflowCtx`, and the HTTP router.

use std::sync::Arc;
use std::time::Duration;

use assay_domain::events::EngineEventBus;
use tracing::info;

use crate::config::{BackendConfig, EngineConfig};

/// One row to seed into `engine.modules` on first boot.
/// `default_enabled = false` means operators must flip it to TRUE
/// before its migrations run — used for opt-in modules like auth so
/// existing v0.1.2 deployments don't get unexpected schema changes.
#[derive(Debug, Clone)]
pub struct BuiltinModule {
    pub name: &'static str,
    pub version: &'static str,
    pub default_enabled: bool,
}

/// Built-in modules implied by the running build's compile-time features.
///
/// Workflow is always-on (the engine is currently the workflow runtime).
/// Auth — when compiled in via the `auth` Cargo feature — seeds disabled
/// so operators of existing v0.1.2 deployments don't get unexpected
/// auth migrations on upgrade. Local dev flips this via
/// `EngineConfig.auto_enable_modules = ["auth"]`.
pub fn builtin_modules() -> Vec<BuiltinModule> {
    vec![
        BuiltinModule {
            name: "workflow",
            version: env!("CARGO_PKG_VERSION"),
            default_enabled: true,
        },
        // engine itself authenticates every admin + workflow request via
        // the auth module; running with auth disabled isn't supported.
        BuiltinModule {
            name: "auth",
            version: env!("CARGO_PKG_VERSION"),
            default_enabled: true,
        },
    ]
}

/// Sweep interval for the stale-instance cleanup task. Removes
/// `engine.instances` rows whose `last_heartbeat` is older than
/// [`INSTANCE_STALE_SECS`].
const INSTANCE_HEARTBEAT_SECS: u64 = 15;
// Used by the PG cleanup task that prunes dead `engine.instances` rows.
// SQLite path is single-instance and never accumulates stale rows.
#[cfg(feature = "backend-postgres")]
const INSTANCE_STALE_SECS: f64 = 60.0;

/// Result of the engine boot sequence — the parts each backend wired up.
/// The engine binary uses these to compose its `WorkflowStore` /
/// `WorkflowCtx` / HTTP router.
pub enum EngineBoot {
    #[cfg(feature = "backend-postgres")]
    Postgres(PgBoot),
    #[cfg(feature = "backend-sqlite")]
    Sqlite(SqliteBoot),
}

#[cfg(feature = "backend-postgres")]
pub struct PgBoot {
    pub pool: sqlx::PgPool,
    pub bus: Arc<dyn EngineEventBus>,
    pub instance_id: uuid::Uuid,
    pub modules: Vec<String>,
}

#[cfg(feature = "backend-sqlite")]
pub struct SqliteBoot {
    pub pool: sqlx::SqlitePool,
    pub bus: Arc<dyn EngineEventBus>,
    pub instance_id: uuid::Uuid,
    pub modules: Vec<String>,
}

impl EngineBoot {
    /// Run the boot sequence end-to-end against the configured backend.
    pub async fn run(cfg: &EngineConfig) -> anyhow::Result<Self> {
        match cfg.backend.clone() {
            #[cfg(feature = "backend-postgres")]
            BackendConfig::Postgres { url } => {
                let boot = pg_boot(&url, &cfg.auto_enable_modules).await?;
                Ok(EngineBoot::Postgres(boot))
            }
            #[cfg(feature = "backend-sqlite")]
            BackendConfig::Sqlite { .. } => {
                let data_dir = cfg
                    .backend
                    .sqlite_data_dir()
                    .expect("sqlite backend yields data_dir");
                let boot = sqlite_boot(&data_dir, &cfg.auto_enable_modules).await?;
                Ok(EngineBoot::Sqlite(boot))
            }
            #[allow(unreachable_patterns)]
            _ => anyhow::bail!("backend not enabled at compile time"),
        }
    }

    pub fn modules(&self) -> &[String] {
        match self {
            #[cfg(feature = "backend-postgres")]
            EngineBoot::Postgres(b) => &b.modules,
            #[cfg(feature = "backend-sqlite")]
            EngineBoot::Sqlite(b) => &b.modules,
        }
    }

    pub fn instance_id(&self) -> uuid::Uuid {
        match self {
            #[cfg(feature = "backend-postgres")]
            EngineBoot::Postgres(b) => b.instance_id,
            #[cfg(feature = "backend-sqlite")]
            EngineBoot::Sqlite(b) => b.instance_id,
        }
    }
}

#[cfg(feature = "backend-postgres")]
async fn pg_boot(url: &str, auto_enable: &[String]) -> anyhow::Result<PgBoot> {
    use assay_domain::engine::PgEngineSchema;
    use assay_domain::events::PgEngineEventBus;
    use sqlx::PgPool;

    info!(target: "assay-engine", "boot: connecting to postgres");
    let pool = PgPool::connect(url)
        .await
        .map_err(|e| anyhow::anyhow!("connect postgres: {e}"))?;

    let schema = PgEngineSchema::new(pool.clone());
    schema
        .migrate()
        .await
        .map_err(|e| anyhow::anyhow!("engine schema migrate (pg): {e}"))?;
    record_engine_migration_pg(&pool, "engine", 1).await?;

    let modules = read_or_seed_modules_pg(&schema, auto_enable).await?;

    // Per-module schema setup. The workflow module's actual DDL still
    // runs inside `PostgresStore::migrate` when the engine binary builds
    // the store — Phase 2 already moved those tables into the `workflow`
    // schema. We just ensure the schema container exists here so a fresh
    // boot doesn't fail before the store's CREATE TABLE runs.
    for name in &modules {
        let create = format!("CREATE SCHEMA IF NOT EXISTS {name}");
        sqlx::query(&create)
            .execute(&pool)
            .await
            .map_err(|e| anyhow::anyhow!("create schema {name}: {e}"))?;
        record_engine_migration_pg(&pool, name, 1).await?;
    }

    // Auth schema migration — always runs (auth is mandatory per
    // boot) and smoke-touches the OIDC provider tables so missing DDL or
    // permission issues surface here rather than at first request.
    if modules.iter().any(|m| m == "auth") {
        assay_auth::schema::migrate_postgres(&pool)
            .await
            .map_err(|e| anyhow::anyhow!("auth schema migrate (pg): {e}"))?;
        let _ = assay_auth::biscuit::load_or_init_postgres(&pool)
            .await
            .map_err(|e| anyhow::anyhow!("biscuit root key bootstrap (pg): {e}"))?;
        sqlx::query("SELECT COUNT(*) FROM auth.oidc_clients")
            .fetch_one(&pool)
            .await
            .map_err(|e| anyhow::anyhow!("oidc provider tables (pg): {e}"))?;
    }

    let bus: Arc<dyn EngineEventBus> = Arc::new(
        PgEngineEventBus::new(pool.clone(), url)
            .await
            .map_err(|e| anyhow::anyhow!("engine-events bus (pg): {e}"))?,
    );

    let instance_id = schema
        .register_instance(&modules, Some(env!("CARGO_PKG_VERSION")))
        .await
        .map_err(|e| anyhow::anyhow!("register engine.instances row: {e}"))?;
    spawn_pg_instance_lifecycle(pool.clone(), instance_id);

    info!(target: "assay-engine", instance = %instance_id, modules = ?modules, "boot complete (pg)");
    Ok(PgBoot {
        pool,
        bus,
        instance_id,
        modules,
    })
}

#[cfg(feature = "backend-postgres")]
async fn read_or_seed_modules_pg(
    schema: &assay_domain::engine::PgEngineSchema,
    auto_enable: &[String],
) -> anyhow::Result<Vec<String>> {
    let existing = schema
        .list_modules()
        .await
        .map_err(|e| anyhow::anyhow!("list engine.modules (pg): {e}"))?;
    let known: std::collections::HashSet<String> =
        existing.iter().map(|m| m.name.clone()).collect();

    // Seed any compile-time module that isn't already in engine.modules.
    // Each module's `default_enabled` is honoured unless the operator
    // explicitly listed it in `auto_enable_modules` — that override
    // exists so local-dev configs can flip auth on without an extra
    // setup step.
    for module in builtin_modules() {
        if known.contains(module.name) {
            continue;
        }
        let enabled = module.default_enabled
            || auto_enable.iter().any(|n| n == module.name);
        schema
            .upsert_module(module.name, Some(module.version), enabled)
            .await
            .map_err(|e| anyhow::anyhow!("seed engine.modules row {}: {e}", module.name))?;
    }

    let final_list = schema
        .list_modules()
        .await
        .map_err(|e| anyhow::anyhow!("re-list engine.modules (pg): {e}"))?;
    Ok(final_list
        .into_iter()
        .filter(|m| m.enabled)
        .map(|m| m.name)
        .collect())
}

#[cfg(feature = "backend-postgres")]
async fn record_engine_migration_pg(
    pool: &sqlx::PgPool,
    module: &str,
    version: i32,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO engine.migrations (module, version)
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(module)
    .bind(version)
    .execute(pool)
    .await
    .map_err(|e| anyhow::anyhow!("record engine.migrations row {module}/{version}: {e}"))?;
    Ok(())
}

#[cfg(feature = "backend-postgres")]
fn spawn_pg_instance_lifecycle(pool: sqlx::PgPool, id: uuid::Uuid) {
    use assay_domain::engine::PgEngineSchema;
    let schema = PgEngineSchema::new(pool.clone());
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(INSTANCE_HEARTBEAT_SECS));
        loop {
            tick.tick().await;
            if let Err(e) = schema.heartbeat_instance(id).await {
                tracing::warn!(?e, %id, "engine.instances heartbeat failed");
            }
            // Best-effort stale cleanup. Idempotent — multiple instances
            // racing the same DELETE is fine.
            let cutoff_sql = format!(
                "DELETE FROM engine.instances
                 WHERE last_heartbeat < EXTRACT(EPOCH FROM NOW()) - {INSTANCE_STALE_SECS}"
            );
            if let Err(e) = sqlx::query(&cutoff_sql).execute(&pool).await {
                tracing::debug!(?e, "engine.instances stale cleanup failed");
            }
        }
    });
}

#[cfg(feature = "backend-sqlite")]
async fn sqlite_boot(data_dir: &str, auto_enable: &[String]) -> anyhow::Result<SqliteBoot> {
    use assay_domain::engine::SqliteEngineSchema;
    use assay_domain::events::SqliteEngineEventBus;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    let in_memory = data_dir == ":memory:";
    if !in_memory {
        std::fs::create_dir_all(data_dir)
            .map_err(|e| anyhow::anyhow!("create data_dir {data_dir}: {e}"))?;
    }

    // The connection's "main" is a transient in-memory router. All real
    // tables live in ATTACHed databases so engine-qualified queries
    // (`engine.events`, `workflow.workflows`) match the PG syntax exactly.
    let main_url = "sqlite::memory:";
    let opts = SqliteConnectOptions::from_str(main_url)?.create_if_missing(true);

    let engine_attach = sqlite_attach_uri(data_dir, "engine", in_memory);
    let workflow_attach = sqlite_attach_uri(data_dir, "workflow", in_memory);
    let auth_attach = sqlite_attach_uri(data_dir, "auth", in_memory);

    info!(
        target: "assay-engine",
        data_dir = %data_dir,
        engine = %engine_attach,
        workflow = %workflow_attach,
        "boot: opening sqlite engine pool"
    );

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(move |conn, _meta| {
            let engine_attach = engine_attach.clone();
            let workflow_attach = workflow_attach.clone();
            let auth_attach = auth_attach.clone();
            Box::pin(async move {
                use sqlx::Executor;
                conn.execute(
                    format!("ATTACH DATABASE '{engine_attach}' AS engine").as_str(),
                )
                .await?;
                conn.execute(
                    format!("ATTACH DATABASE '{workflow_attach}' AS workflow").as_str(),
                )
                .await?;
                conn.execute(
                    format!("ATTACH DATABASE '{auth_attach}' AS auth").as_str(),
                )
                .await?;
                Ok(())
            })
        })
        .connect_with(opts)
        .await
        .map_err(|e| anyhow::anyhow!("connect sqlite: {e}"))?;

    let schema = SqliteEngineSchema::new(pool.clone());
    schema
        .migrate()
        .await
        .map_err(|e| anyhow::anyhow!("engine schema migrate (sqlite): {e}"))?;
    record_engine_migration_sqlite(&pool, "engine", 1).await?;

    let modules = read_or_seed_modules_sqlite(&schema, auto_enable).await?;
    for name in &modules {
        record_engine_migration_sqlite(&pool, name, 1).await?;
    }

    // Auth schema migration — always runs (auth is mandatory per
    if modules.iter().any(|m| m == "auth") {
        assay_auth::schema::migrate_sqlite(&pool)
            .await
            .map_err(|e| anyhow::anyhow!("auth schema migrate (sqlite): {e}"))?;
        let _ = assay_auth::biscuit::load_or_init_sqlite(&pool)
            .await
            .map_err(|e| anyhow::anyhow!("biscuit root key bootstrap (sqlite): {e}"))?;
        sqlx::query("SELECT COUNT(*) FROM auth.oidc_clients")
            .fetch_one(&pool)
            .await
            .map_err(|e| anyhow::anyhow!("oidc provider tables (sqlite): {e}"))?;
    }

    let bus: Arc<dyn EngineEventBus> = Arc::new(
        SqliteEngineEventBus::new(pool.clone())
            .await
            .map_err(|e| anyhow::anyhow!("engine-events bus (sqlite): {e}"))?,
    );

    let instance_id = schema
        .register_instance(&modules, Some(env!("CARGO_PKG_VERSION")))
        .await
        .map_err(|e| anyhow::anyhow!("register engine.instances row: {e}"))?;
    spawn_sqlite_instance_lifecycle(pool.clone(), instance_id);

    info!(target: "assay-engine", instance = %instance_id, modules = ?modules, "boot complete (sqlite)");
    Ok(SqliteBoot {
        pool,
        bus,
        instance_id,
        modules,
    })
}

#[cfg(feature = "backend-sqlite")]
fn sqlite_attach_uri(data_dir: &str, module: &str, in_memory: bool) -> String {
    if in_memory {
        // Shared-cache memdb so every connection in the pool sees the
        // same in-memory tables, and so reopening the pool after process
        // restart picks up the fresh DB. Per-process suffix avoids
        // collisions when multiple engines run in the same test binary.
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let suffix = format!(
            "{}_{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        format!("file:assay_{module}_{suffix}?mode=memory&cache=shared")
    } else {
        format!("file:{data_dir}/{module}.db?mode=rwc")
    }
}

#[cfg(feature = "backend-sqlite")]
async fn read_or_seed_modules_sqlite(
    schema: &assay_domain::engine::SqliteEngineSchema,
    auto_enable: &[String],
) -> anyhow::Result<Vec<String>> {
    let existing = schema
        .list_modules()
        .await
        .map_err(|e| anyhow::anyhow!("list engine.modules (sqlite): {e}"))?;
    let known: std::collections::HashSet<String> =
        existing.iter().map(|m| m.name.clone()).collect();

    // Same per-module insert pattern as the PG path: skip rows that
    // already exist, honour `default_enabled` unless the operator
    // explicitly auto-enabled the module.
    for module in builtin_modules() {
        if known.contains(module.name) {
            continue;
        }
        let enabled = module.default_enabled
            || auto_enable.iter().any(|n| n == module.name);
        schema
            .upsert_module(module.name, Some(module.version), enabled)
            .await
            .map_err(|e| anyhow::anyhow!("seed engine.modules row {}: {e}", module.name))?;
    }

    let final_list = schema
        .list_modules()
        .await
        .map_err(|e| anyhow::anyhow!("re-list engine.modules (sqlite): {e}"))?;
    Ok(final_list
        .into_iter()
        .filter(|m| m.enabled)
        .map(|m| m.name)
        .collect())
}

#[cfg(feature = "backend-sqlite")]
async fn record_engine_migration_sqlite(
    pool: &sqlx::SqlitePool,
    module: &str,
    version: i32,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO engine.migrations (module, version)
         VALUES (?, ?)",
    )
    .bind(module)
    .bind(version)
    .execute(pool)
    .await
    .map_err(|e| anyhow::anyhow!("record engine.migrations row {module}/{version}: {e}"))?;
    Ok(())
}

#[cfg(feature = "backend-sqlite")]
fn spawn_sqlite_instance_lifecycle(pool: sqlx::SqlitePool, id: uuid::Uuid) {
    use assay_domain::engine::SqliteEngineSchema;
    let schema = SqliteEngineSchema::new(pool);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(INSTANCE_HEARTBEAT_SECS));
        loop {
            tick.tick().await;
            if let Err(e) = schema.heartbeat_instance(id).await {
                tracing::warn!(?e, %id, "engine.instances heartbeat failed");
            }
        }
    });
}

#[cfg(all(test, feature = "backend-sqlite"))]
mod tests {
    use super::*;

    /// Plan-15 slice 3: auth is default-enabled. The `auto_enable_modules`
    /// argument is a no-op for auth (kept as a setting for forward-compat
    /// with future opt-in modules) — auth always runs its migration on
    /// first boot now.
    #[tokio::test(flavor = "multi_thread")]
    async fn sqlite_boot_default_runs_auth_migration() {
        let boot = sqlite_boot(":memory:", &[]).await.expect("boot");
        assert!(
            boot.modules.iter().any(|m| m == "auth"),
            "auth must be in active modules by default; got {:?}",
            boot.modules
        );
        // Auth migration recorded.
        let auth_row: Option<(String,)> = sqlx::query_as(
            "SELECT module FROM engine.migrations WHERE module = 'auth'",
        )
        .fetch_optional(&boot.pool)
        .await
        .expect("query engine.migrations");
        assert!(
            auth_row.is_some(),
            "engine.migrations should have an auth row after auto-enabled boot"
        );
        // auth.users table should exist (proves migrate_sqlite ran against
        // the ATTACHed auth db).
        let user_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM auth.users")
                .fetch_one(&boot.pool)
                .await
                .expect("count auth.users");
        assert_eq!(user_count.0, 0);
    }
}
