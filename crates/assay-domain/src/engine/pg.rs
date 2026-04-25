//! Postgres implementation of the engine-core schema.
//!
//! Creates the `engine` schema and the four engine-scope tables on
//! first boot, idempotently. PG18 is the minimum supported version
//! (see plan 12 Principle 7); we lean on `uuidv7()` for primary keys
//! that need temporal ordering.

use anyhow::{Context, Result};
use sqlx::PgPool;

use super::ModuleRecord;

/// Bootstrap + read interface for the engine-core PG schema.
pub struct PgEngineSchema {
    pool: PgPool,
}

impl PgEngineSchema {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create the `engine` schema and engine-core tables. Idempotent.
    pub async fn migrate(&self) -> Result<()> {
        // Engine schema lives unconditionally — it's the home for the
        // module manifest, audit log, instance registry, and migration
        // tracker. Other module schemas (workflow, auth, …) are created
        // by their own bootstrap when enabled.
        sqlx::query("CREATE SCHEMA IF NOT EXISTS engine")
            .execute(&self.pool)
            .await
            .context("create engine schema")?;

        // Bootstrap the migrations table first so future schema changes
        // can record themselves. PRIMARY KEY (module, version) lets each
        // module advance independently.
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS engine.migrations (
                module       TEXT NOT NULL,
                version      INTEGER NOT NULL,
                applied_at   DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
                PRIMARY KEY (module, version)
            )",
        )
        .execute(&self.pool)
        .await
        .context("create engine.migrations")?;

        // Module manifest. `config` is a JSONB blob the module owns —
        // engine just stores it. `enabled_by` is informational
        // (operator id / audit subject) and may be NULL.
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
        .execute(&self.pool)
        .await
        .context("create engine.modules")?;

        // Engine-level operations log. Append-only by convention; no
        // delete API. PG18's native uuidv7() gives us temporally
        // sortable ids without an extra index.
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS engine.audit (
                id           UUID PRIMARY KEY DEFAULT uuidv7(),
                ts           DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
                actor        TEXT,
                action       TEXT NOT NULL,
                details      JSONB NOT NULL DEFAULT '{}'::jsonb
            )",
        )
        .execute(&self.pool)
        .await
        .context("create engine.audit")?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_engine_audit_ts ON engine.audit(ts)")
            .execute(&self.pool)
            .await
            .context("create idx_engine_audit_ts")?;

        // Live engine processes. Visibility-only in v0.1.2 — coordination
        // (leader election, task claiming) keeps using
        // pg_try_advisory_lock + FOR UPDATE SKIP LOCKED. A stale row
        // here doesn't break the engine; the dashboard "instances" view
        // (v0.14.0) will display these.
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS engine.instances (
                id              UUID PRIMARY KEY DEFAULT uuidv7(),
                started_at      DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
                last_heartbeat  DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
                namespaces      TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
                version         TEXT
            )",
        )
        .execute(&self.pool)
        .await
        .context("create engine.instances")?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_engine_instances_heartbeat \
             ON engine.instances(last_heartbeat)",
        )
        .execute(&self.pool)
        .await
        .context("create idx_engine_instances_heartbeat")?;

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
