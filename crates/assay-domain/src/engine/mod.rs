//! Engine-core schema and bootstrap.
//!
//! Owns the four engine-scope tables introduced in v0.1.2:
//! `engine.modules`, `engine.audit`, `engine.instances`, `engine.migrations`.
//!
//! These tables are engine-core infrastructure and are always present
//! regardless of which functional modules are enabled. Module-specific
//! schemas (`workflow`, `auth`, …) are created/skipped based on
//! `engine.modules` enablement.
//!
//! ## Backend layout
//!
//! - **Postgres**: tables live in the `engine` schema, addressed
//!   schema-qualified (`engine.modules`, etc.).
//! - **SQLite**: tables live in an `engine.db` file attached as the
//!   `engine` database (Phase 3+); names match the PG layout exactly so
//!   queries are identical.

#[cfg(feature = "backend-postgres")]
pub mod pg;

#[cfg(feature = "backend-postgres")]
pub use pg::PgEngineSchema;

#[cfg(feature = "backend-sqlite")]
pub mod sqlite;

#[cfg(feature = "backend-sqlite")]
pub use sqlite::SqliteEngineSchema;

/// Advisory-lock id held by every Postgres schema migration here.
///
/// `CREATE ... IF NOT EXISTS` is not atomic in Postgres: the existence
/// check precedes the catalog insert, so concurrent boots both pass the
/// check and one loses on `pg_namespace_nspname_index`. Modules share
/// one id because they share the `CREATE SCHEMA` statements.
#[cfg(feature = "backend-postgres")]
pub const SCHEMA_MIGRATION_LOCK: i64 = 0x6173_7361_795f_656e;

/// Take [`SCHEMA_MIGRATION_LOCK`] for the rest of `conn`'s transaction.
/// Transaction-scoped, so commit, rollback and a dropped connection all
/// release it. Callers must run their DDL on the same transaction.
#[cfg(feature = "backend-postgres")]
pub async fn acquire_schema_lock(conn: &mut sqlx::PgConnection) -> sqlx::Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(SCHEMA_MIGRATION_LOCK)
        .execute(conn)
        .await?;
    Ok(())
}

/// Postgres codes for "this object was created while I was creating it":
/// unique violation on a catalog index, duplicate table, duplicate object.
#[cfg(feature = "backend-postgres")]
const DDL_CONFLICT_CODES: &[&str] = &["23505", "42P07", "42710"];

/// Whether `err` is Postgres refusing DDL because something else won the
/// same `CREATE ... IF NOT EXISTS`.
#[cfg(feature = "backend-postgres")]
pub fn is_ddl_conflict(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<sqlx::Error>()
            .and_then(|e| e.as_database_error())
            .and_then(|db| db.code())
            .is_some_and(|code| DDL_CONFLICT_CODES.contains(&code.as_ref()))
    })
}

/// Run a schema migration, retrying when a concurrent one beat it to an
/// object. [`acquire_schema_lock`] is the real defence; this covers
/// callers that reach the catalog without it. A conflict aborts the
/// whole transaction, so the retry re-runs the closure from the top.
#[cfg(feature = "backend-postgres")]
pub async fn retry_ddl<F, Fut, T>(attempts: usize, mut migrate: F) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    let mut last: Option<anyhow::Error> = None;
    for attempt in 1..=attempts.max(1) {
        match migrate().await {
            Ok(value) => return Ok(value),
            Err(err) if is_ddl_conflict(&err) && attempt < attempts => {
                tracing::warn!(
                    attempt,
                    error = %err,
                    "another engine created this object first; retrying schema setup"
                );
                last = Some(err);
            }
            Err(err) => return Err(err),
        }
    }
    Err(last.expect("the loop runs at least once"))
}

/// A row from the `engine.modules` table — the boot manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleRecord {
    pub name: String,
    pub enabled: bool,
    pub enabled_at: Option<f64>,
    pub enabled_by: Option<String>,
    pub version: Option<String>,
    pub config: serde_json::Value,
}

/// A row from the `engine.audit` table — append-only operations log.
/// Surfaced through the engine dashboard's audit pane and the
/// `/api/v1/engine/audit` admin endpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditRecord {
    pub id: String,
    pub ts: f64,
    pub actor: Option<String>,
    pub action: String,
    pub details: serde_json::Value,
}

/// A row from the `engine.instances` table — live engine processes
/// registered at boot. Multi-node visibility for the dashboard's
/// instances pane.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceRecord {
    pub id: String,
    pub started_at: f64,
    pub last_heartbeat: f64,
    pub namespaces: Vec<String>,
    pub version: Option<String>,
}
