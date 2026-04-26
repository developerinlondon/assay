//! Postgres implementation of the engine-core schema.
//!
//! Creates the `engine` schema and the four engine-scope tables on
//! first boot, idempotently. PG18 is the minimum supported version
//! (see plan 12 Principle 7); we lean on `uuidv7()` for primary keys
//! that need temporal ordering.

use anyhow::{Context, Result};
use sqlx::PgPool;

use super::{AuditRecord, InstanceRecord, ModuleRecord};

/// Bootstrap + read interface for the engine-core PG schema.
pub struct PgEngineSchema {
    pool: PgPool,
}

impl PgEngineSchema {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create the `engine` schema and engine-core tables. Idempotent
    /// AND race-safe across concurrent callers (test parallelism +
    /// multi-instance production boot both rely on this).
    ///
    /// PG's `CREATE SCHEMA IF NOT EXISTS` is **not** race-free against
    /// `pg_namespace`: two callers can both pass the existence check
    /// then both INSERT, and one loses with
    /// `duplicate key value violates unique constraint
    /// "pg_namespace_nspname_index"`. Same applies to concurrent
    /// `CREATE TABLE IF NOT EXISTS` against `pg_class`.
    ///
    /// The fix is to serialise concurrent migrate() calls via a
    /// transaction-scoped advisory lock. The lock is held only for the
    /// duration of the migration transaction; subsequent boots that
    /// find the schema already present pay one fast SELECT for the
    /// lock then a no-op pass through every `IF NOT EXISTS`.
    pub async fn migrate(&self) -> Result<()> {
        // Stable lock id — hash of "assay-engine-schema-migration"
        // truncated to i64. Different from any other advisory lock id
        // the engine uses (workflow uses 1; this is namespaced).
        const ENGINE_MIGRATION_LOCK: i64 = 0x6173_7361_795f_656e; // "assay_en"

        let mut tx = self.pool.begin().await.context("begin migrate tx")?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(ENGINE_MIGRATION_LOCK)
            .execute(&mut *tx)
            .await
            .context("acquire engine migration advisory lock")?;

        // Engine schema lives unconditionally — it's the home for the
        // module manifest, audit log, instance registry, and migration
        // tracker. Other module schemas (workflow, auth, …) are created
        // by their own bootstrap when enabled.
        sqlx::query("CREATE SCHEMA IF NOT EXISTS engine")
            .execute(&mut *tx)
            .await
            .context("create engine schema")?;

        // All DDL runs against the same transaction that holds the
        // advisory lock above — that's what makes the migration
        // race-safe. CREATE TABLE IF NOT EXISTS otherwise has the
        // same pg_class race as CREATE SCHEMA does on pg_namespace.
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS engine.migrations (
                module       TEXT NOT NULL,
                version      INTEGER NOT NULL,
                applied_at   DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
                PRIMARY KEY (module, version)
            )",
        )
        .execute(&mut *tx)
        .await
        .context("create engine.migrations")?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS engine.modules (
                name         TEXT PRIMARY KEY,
                enabled      BOOLEAN NOT NULL DEFAULT FALSE,
                enabled_at   DOUBLE PRECISION,
                enabled_by   TEXT,
                version      TEXT,
                config       JSONB NOT NULL DEFAULT '{}'::jsonb
            )",
        )
        .execute(&mut *tx)
        .await
        .context("create engine.modules")?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS engine.audit (
                id           UUID PRIMARY KEY DEFAULT uuidv7(),
                ts           DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
                actor        TEXT,
                action       TEXT NOT NULL,
                details      JSONB NOT NULL DEFAULT '{}'::jsonb
            )",
        )
        .execute(&mut *tx)
        .await
        .context("create engine.audit")?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_engine_audit_ts ON engine.audit(ts)")
            .execute(&mut *tx)
            .await
            .context("create idx_engine_audit_ts")?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS engine.instances (
                id              UUID PRIMARY KEY DEFAULT uuidv7(),
                started_at      DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
                last_heartbeat  DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
                namespaces      TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
                version         TEXT
            )",
        )
        .execute(&mut *tx)
        .await
        .context("create engine.instances")?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_engine_instances_heartbeat \
             ON engine.instances(last_heartbeat)",
        )
        .execute(&mut *tx)
        .await
        .context("create idx_engine_instances_heartbeat")?;

        // Commit releases the advisory lock + makes all the DDL
        // visible atomically.
        tx.commit().await.context("commit migrate tx")?;
        Ok(())
    }

    /// Read the boot manifest. Returns every module row regardless of
    /// `enabled` — callers (engine boot) typically filter to enabled.
    pub async fn list_modules(&self) -> Result<Vec<ModuleRecord>> {
        // Tuple shape of one `engine.modules` row as fetched from PG.
        // Aliased so the query annotation stays readable; sqlx requires
        // the explicit shape for `query_as` row decoding.
        type PgModuleRow = (
            String,
            bool,
            Option<f64>,
            Option<String>,
            Option<String>,
            serde_json::Value,
        );
        let rows: Vec<PgModuleRow> = sqlx::query_as(
            "SELECT name, enabled, enabled_at, enabled_by, version, config
             FROM engine.modules ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await
        .context("list engine.modules")?;
        Ok(rows
            .into_iter()
            .map(
                |(name, enabled, enabled_at, enabled_by, version, config)| ModuleRecord {
                    name,
                    enabled,
                    enabled_at,
                    enabled_by,
                    version,
                    config,
                },
            )
            .collect())
    }

    /// Insert (or no-op if present) a module row, marking it enabled.
    /// Used by engine boot to seed the manifest based on `engine.toml`.
    pub async fn upsert_module(
        &self,
        name: &str,
        version: Option<&str>,
        enabled: bool,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO engine.modules (name, enabled, enabled_at, version)
             VALUES ($1, $2, EXTRACT(EPOCH FROM NOW()), $3)
             ON CONFLICT (name) DO UPDATE
                SET enabled = EXCLUDED.enabled,
                    enabled_at = COALESCE(engine.modules.enabled_at, EXCLUDED.enabled_at),
                    version = EXCLUDED.version",
        )
        .bind(name)
        .bind(enabled)
        .bind(version)
        .execute(&self.pool)
        .await
        .context("upsert engine.modules row")?;
        Ok(())
    }

    /// Append an audit log entry. Append-only by design.
    pub async fn audit(
        &self,
        actor: Option<&str>,
        action: &str,
        details: &serde_json::Value,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO engine.audit (actor, action, details) VALUES ($1, $2, $3)",
        )
        .bind(actor)
        .bind(action)
        .bind(details)
        .execute(&self.pool)
        .await
        .context("insert engine.audit row")?;
        Ok(())
    }

    /// Register a live engine process. Returns the instance id (UUID).
    pub async fn register_instance(
        &self,
        namespaces: &[String],
        version: Option<&str>,
    ) -> Result<uuid::Uuid> {
        let row: (uuid::Uuid,) = sqlx::query_as(
            "INSERT INTO engine.instances (namespaces, version)
             VALUES ($1, $2) RETURNING id",
        )
        .bind(namespaces)
        .bind(version)
        .fetch_one(&self.pool)
        .await
        .context("register engine.instances row")?;
        Ok(row.0)
    }

    /// Refresh the heartbeat on an instance row. Called periodically.
    pub async fn heartbeat_instance(&self, id: uuid::Uuid) -> Result<()> {
        sqlx::query(
            "UPDATE engine.instances
             SET last_heartbeat = EXTRACT(EPOCH FROM NOW())
             WHERE id = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .context("heartbeat engine.instances row")?;
        Ok(())
    }

    /// Delete the instance row on graceful shutdown.
    pub async fn deregister_instance(&self, id: uuid::Uuid) -> Result<()> {
        sqlx::query("DELETE FROM engine.instances WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .context("deregister engine.instances row")?;
        Ok(())
    }

    /// List engine.audit rows newest-first with the configured page
    /// window. Optional filters narrow by exact actor / action / time
    /// bounds. Returned `details` is the parsed JSONB blob.
    pub async fn list_audit(
        &self,
        limit: i64,
        offset: i64,
        actor: Option<&str>,
        action: Option<&str>,
        since: Option<f64>,
        until: Option<f64>,
    ) -> Result<(Vec<AuditRecord>, i64)> {
        // Build a single dynamic WHERE clause shared by SELECT + COUNT so
        // the page + total stay consistent. Bound positions ($1..) are
        // assigned in the order arguments are pushed.
        let mut clauses: Vec<String> = Vec::new();
        let mut binds: Vec<DynBind> = Vec::new();
        if let Some(a) = actor {
            clauses.push(format!("actor = ${}", binds.len() + 1));
            binds.push(DynBind::Text(a.to_string()));
        }
        if let Some(a) = action {
            clauses.push(format!("action = ${}", binds.len() + 1));
            binds.push(DynBind::Text(a.to_string()));
        }
        if let Some(t) = since {
            clauses.push(format!("ts >= ${}", binds.len() + 1));
            binds.push(DynBind::Float(t));
        }
        if let Some(t) = until {
            clauses.push(format!("ts <= ${}", binds.len() + 1));
            binds.push(DynBind::Float(t));
        }
        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };

        let select_sql = format!(
            "SELECT id, ts, actor, action, details
             FROM engine.audit
             {where_sql}
             ORDER BY ts DESC, id DESC
             LIMIT ${} OFFSET ${}",
            binds.len() + 1,
            binds.len() + 2,
        );
        type AuditPgRow = (uuid::Uuid, f64, Option<String>, String, serde_json::Value);
        let mut q = sqlx::query_as::<_, AuditPgRow>(&select_sql);
        for b in &binds {
            q = match b {
                DynBind::Text(s) => q.bind(s),
                DynBind::Float(f) => q.bind(*f),
            };
        }
        let rows: Vec<AuditPgRow> = q
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .context("list engine.audit page")?;

        let count_sql = format!("SELECT COUNT(*) FROM engine.audit {where_sql}");
        let mut cq = sqlx::query_as::<_, (i64,)>(&count_sql);
        for b in &binds {
            cq = match b {
                DynBind::Text(s) => cq.bind(s),
                DynBind::Float(f) => cq.bind(*f),
            };
        }
        let total: (i64,) = cq
            .fetch_one(&self.pool)
            .await
            .context("count engine.audit page")?;

        let items = rows
            .into_iter()
            .map(|(id, ts, actor, action, details)| AuditRecord {
                id: id.to_string(),
                ts,
                actor,
                action,
                details,
            })
            .collect();
        Ok((items, total.0))
    }

    /// List currently-registered engine instances ordered by most-recent
    /// heartbeat first. Stale rows (over INSTANCE_STALE_SECS without a
    /// heartbeat) are pruned by the boot loop's cleanup task; this read
    /// returns whatever's currently present.
    pub async fn list_instances(&self) -> Result<Vec<InstanceRecord>> {
        type InstancePgRow = (uuid::Uuid, f64, f64, Vec<String>, Option<String>);
        let rows: Vec<InstancePgRow> = sqlx::query_as(
            "SELECT id, started_at, last_heartbeat, namespaces, version
             FROM engine.instances
             ORDER BY last_heartbeat DESC",
        )
        .fetch_all(&self.pool)
        .await
        .context("list engine.instances")?;
        Ok(rows
            .into_iter()
            .map(|(id, started_at, last_heartbeat, namespaces, version)| InstanceRecord {
                id: id.to_string(),
                started_at,
                last_heartbeat,
                namespaces,
                version,
            })
            .collect())
    }

    /// Toggle a module row's `enabled` flag. Records the actor in
    /// `enabled_by` so the audit log can correlate operator actions
    /// without an extra column. Returns `Ok(false)` when the module
    /// doesn't exist (caller surfaces 404), `Ok(true)` on a flip.
    pub async fn set_module_enabled(
        &self,
        name: &str,
        enabled: bool,
        actor: Option<&str>,
    ) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE engine.modules
             SET enabled = $1,
                 enabled_at = EXTRACT(EPOCH FROM NOW()),
                 enabled_by = COALESCE($2, enabled_by)
             WHERE name = $3",
        )
        .bind(enabled)
        .bind(actor)
        .bind(name)
        .execute(&self.pool)
        .await
        .context("set engine.modules.enabled")?;
        Ok(res.rows_affected() > 0)
    }
}

/// Dynamic-bind helper for [`PgEngineSchema::list_audit`]. The
/// filter set is small and stable; an enum keeps the bind ordering
/// trivial without pulling in a query builder.
enum DynBind {
    Text(String),
    Float(f64),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db_url() -> Option<String> {
        std::env::var("TEST_DATABASE_URL")
            .or_else(|_| std::env::var("ASSAY_PG_TEST_URL"))
            .ok()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn migrate_creates_tables_and_round_trip() {
        let Some(url) = test_db_url() else {
            eprintln!("skipped: TEST_DATABASE_URL not set");
            return;
        };
        let pool = PgPool::connect(&url).await.unwrap();
        let schema = PgEngineSchema::new(pool);

        // Idempotent: two migrate() calls in a row work.
        schema.migrate().await.unwrap();
        schema.migrate().await.unwrap();

        // Module round-trip
        schema
            .upsert_module("test_workflow", Some("0.2.2"), true)
            .await
            .unwrap();
        let mods = schema.list_modules().await.unwrap();
        assert!(mods.iter().any(|m| m.name == "test_workflow" && m.enabled));

        // Audit append
        schema
            .audit(Some("op-1"), "test_action", &serde_json::json!({"k": "v"}))
            .await
            .unwrap();

        // Instance lifecycle
        let id = schema
            .register_instance(&["main".to_string()], Some("0.1.2"))
            .await
            .unwrap();
        schema.heartbeat_instance(id).await.unwrap();
        schema.deregister_instance(id).await.unwrap();

        // Cleanup
        sqlx::query("DELETE FROM engine.modules WHERE name = 'test_workflow'")
            .execute(&schema.pool)
            .await
            .unwrap();
    }
}
