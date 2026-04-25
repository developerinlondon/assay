//! SQLite implementation of the engine-core schema.
//!
//! In v0.1.2 Phase 3+, the engine opens `engine.db` and ATTACHes
//! per-module `.db` files. This module's `migrate()` runs against the
//! `engine` database (the attached or main DB that owns the engine
//! schema) and creates the four engine-scope tables there.
//!
//! Table names match the PG schema-qualified layout exactly: callers
//! address them as `engine.modules`, `engine.audit`, `engine.instances`,
//! `engine.migrations` regardless of backend (SQLite ATTACH makes the
//! syntax work on the SQLite side; PG's schema does on the PG side).
//!
//! UUIDv7 is generated in Rust (the `uuid` crate v1.18 supports v7) so
//! we don't depend on a SQLite extension; we bind the value as a TEXT
//! representation since SQLite has no native UUID type.

use anyhow::{Context, Result};
use sqlx::SqlitePool;

use super::ModuleRecord;

pub struct SqliteEngineSchema {
    pool: SqlitePool,
    /// Schema/database name to qualify queries with. Defaults to
    /// `engine` (matches the ATTACHed alias used by the engine init).
    /// Tests that own their own pool can pass `main` to point at the
    /// default database.
    schema: String,
}

impl SqliteEngineSchema {
    /// Construct against an attached `engine` database.
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            schema: "engine".to_string(),
        }
    }

    /// Construct against the `main` database (no ATTACH). Used by tests
    /// and by Phase 1 fallbacks where ATTACH hasn't been wired yet.
    pub fn new_in_main(pool: SqlitePool) -> Self {
        Self {
            pool,
            schema: "main".to_string(),
        }
    }

    fn q(&self, table: &str) -> String {
        format!("{}.{}", self.schema, table)
    }

    /// Create the engine-core tables on the configured schema. Idempotent.
    pub async fn migrate(&self) -> Result<()> {
        // Bootstrap migrations table first so future schema bumps can
        // record themselves.
        sqlx::query(&format!(
            "CREATE TABLE IF NOT EXISTS {} (
                module       TEXT NOT NULL,
                version      INTEGER NOT NULL,
                applied_at   REAL NOT NULL DEFAULT (CAST(strftime('%s','now') AS REAL)),
                PRIMARY KEY (module, version)
            )",
            self.q("migrations")
        ))
        .execute(&self.pool)
        .await
        .context("create engine.migrations")?;

        sqlx::query(&format!(
            "CREATE TABLE IF NOT EXISTS {} (
                name         TEXT PRIMARY KEY,
                enabled      INTEGER NOT NULL DEFAULT 0,
                enabled_at   REAL,
                enabled_by   TEXT,
                version      TEXT,
                config       TEXT NOT NULL DEFAULT '{{}}'
            )",
            self.q("modules")
        ))
        .execute(&self.pool)
        .await
        .context("create engine.modules")?;

        // Audit log — UUIDv7 generated in Rust on insert; stored as TEXT.
        sqlx::query(&format!(
            "CREATE TABLE IF NOT EXISTS {} (
                id           TEXT PRIMARY KEY,
                ts           REAL NOT NULL DEFAULT (CAST(strftime('%s','now') AS REAL)),
                actor        TEXT,
                action       TEXT NOT NULL,
                details      TEXT NOT NULL DEFAULT '{{}}'
            )",
            self.q("audit")
        ))
        .execute(&self.pool)
        .await
        .context("create engine.audit")?;
        // SQLite CREATE INDEX requires the index name to be qualified with
        // the same schema as the table; the table reference itself is
        // always unqualified. Build the index name with schema prefix so
        // ATTACH-ed engine.db indexes don't clash with main-db indexes.
        sqlx::query(&format!(
            "CREATE INDEX IF NOT EXISTS {}.idx_engine_audit_ts ON audit(ts)",
            self.schema
        ))
        .execute(&self.pool)
        .await
        .context("create idx_engine_audit_ts")?;

        // SQLite is single-instance so engine.instances is mostly a stub
        // here, but keep the same shape so the table exists for parity
        // with PG (a query against it on either backend returns a row
        // shape callers can rely on).
        sqlx::query(&format!(
            "CREATE TABLE IF NOT EXISTS {} (
                id              TEXT PRIMARY KEY,
                started_at      REAL NOT NULL DEFAULT (CAST(strftime('%s','now') AS REAL)),
                last_heartbeat  REAL NOT NULL DEFAULT (CAST(strftime('%s','now') AS REAL)),
                namespaces      TEXT NOT NULL DEFAULT '[]',
                version         TEXT
            )",
            self.q("instances")
        ))
        .execute(&self.pool)
        .await
        .context("create engine.instances")?;
        sqlx::query(&format!(
            "CREATE INDEX IF NOT EXISTS {}.idx_engine_instances_heartbeat \
             ON instances(last_heartbeat)",
            self.schema
        ))
        .execute(&self.pool)
        .await
        .context("create idx_engine_instances_heartbeat")?;

        Ok(())
    }

    pub async fn list_modules(&self) -> Result<Vec<ModuleRecord>> {
        let sql = format!(
            "SELECT name, enabled, enabled_at, enabled_by, version, config
             FROM {} ORDER BY name",
            self.q("modules")
        );
        // Tuple shape of one `engine.modules` row as fetched from SQLite
        // (booleans stored as INTEGER, JSONB-equivalent as TEXT).
        type SqliteModuleRow = (String, i64, Option<f64>, Option<String>, Option<String>, String);
        let rows: Vec<SqliteModuleRow> =
            sqlx::query_as(&sql)
                .fetch_all(&self.pool)
                .await
                .context("list engine.modules")?;
        Ok(rows
            .into_iter()
            .map(
                |(name, enabled, enabled_at, enabled_by, version, config)| ModuleRecord {
                    name,
                    enabled: enabled != 0,
                    enabled_at,
                    enabled_by,
                    version,
                    config: serde_json::from_str(&config).unwrap_or(serde_json::Value::Null),
                },
            )
            .collect())
    }

    pub async fn upsert_module(
        &self,
        name: &str,
        version: Option<&str>,
        enabled: bool,
    ) -> Result<()> {
        let sql = format!(
            "INSERT INTO {} (name, enabled, enabled_at, version)
             VALUES (?, ?, CAST(strftime('%s','now') AS REAL), ?)
             ON CONFLICT(name) DO UPDATE
                SET enabled = excluded.enabled,
                    enabled_at = COALESCE({}.enabled_at, excluded.enabled_at),
                    version = excluded.version",
            self.q("modules"),
            self.q("modules")
        );
        sqlx::query(&sql)
            .bind(name)
            .bind(if enabled { 1i64 } else { 0i64 })
            .bind(version)
            .execute(&self.pool)
            .await
            .context("upsert engine.modules row")?;
        Ok(())
    }

    pub async fn audit(
        &self,
        actor: Option<&str>,
        action: &str,
        details: &serde_json::Value,
    ) -> Result<()> {
        let id = uuid::Uuid::now_v7().to_string();
        let sql = format!(
            "INSERT INTO {} (id, actor, action, details) VALUES (?, ?, ?, ?)",
            self.q("audit")
        );
        sqlx::query(&sql)
            .bind(id)
            .bind(actor)
            .bind(action)
            .bind(details.to_string())
            .execute(&self.pool)
            .await
            .context("insert engine.audit row")?;
        Ok(())
    }

    pub async fn register_instance(
        &self,
        namespaces: &[String],
        version: Option<&str>,
    ) -> Result<uuid::Uuid> {
        let id = uuid::Uuid::now_v7();
        let ns_json = serde_json::to_string(namespaces).unwrap_or_else(|_| "[]".to_string());
        let sql = format!(
            "INSERT INTO {} (id, namespaces, version) VALUES (?, ?, ?)",
            self.q("instances")
        );
        sqlx::query(&sql)
            .bind(id.to_string())
            .bind(ns_json)
            .bind(version)
            .execute(&self.pool)
            .await
            .context("register engine.instances row")?;
        Ok(id)
    }

    pub async fn heartbeat_instance(&self, id: uuid::Uuid) -> Result<()> {
        let sql = format!(
            "UPDATE {} SET last_heartbeat = CAST(strftime('%s','now') AS REAL) WHERE id = ?",
            self.q("instances")
        );
        sqlx::query(&sql)
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .context("heartbeat engine.instances row")?;
        Ok(())
    }

    pub async fn deregister_instance(&self, id: uuid::Uuid) -> Result<()> {
        let sql = format!("DELETE FROM {} WHERE id = ?", self.q("instances"));
        sqlx::query(&sql)
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .context("deregister engine.instances row")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::new()
            .filename(":memory:")
            .create_if_missing(true);
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn migrate_creates_tables_and_round_trip() {
        let pool = fresh_pool().await;
        let schema = SqliteEngineSchema::new_in_main(pool);
        schema.migrate().await.unwrap();

        // Idempotent: second migrate is a no-op.
        schema.migrate().await.unwrap();

        // Modules round-trip
        schema.upsert_module("workflow", Some("0.2.2"), true).await.unwrap();
        let mods = schema.list_modules().await.unwrap();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name, "workflow");
        assert!(mods[0].enabled);
        assert_eq!(mods[0].version.as_deref(), Some("0.2.2"));

        // Audit
        schema
            .audit(Some("op-1"), "module_enabled", &serde_json::json!({"name":"workflow"}))
            .await
            .unwrap();

        // Instances
        let id = schema
            .register_instance(&["main".to_string()], Some("0.1.2"))
            .await
            .unwrap();
        schema.heartbeat_instance(id).await.unwrap();
        schema.deregister_instance(id).await.unwrap();
    }
}
