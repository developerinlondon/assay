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

use super::{AuditRecord, InstanceRecord, ModuleRecord};

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

    /// List engine.audit rows newest-first with the configured page
    /// window. Mirrors the PG `list_audit` shape exactly so the engine
    /// dashboard's audit pane uses one client. Returned `details` is the
    /// parsed JSON blob (TEXT in SQLite, JSONB in PG).
    pub async fn list_audit(
        &self,
        limit: i64,
        offset: i64,
        actor: Option<&str>,
        action: Option<&str>,
        since: Option<f64>,
        until: Option<f64>,
    ) -> Result<(Vec<AuditRecord>, i64)> {
        // Build the same dynamic-WHERE both queries share. SQLite uses
        // positional `?` binds; we just push values into a typed enum
        // so the order matches the SQL.
        let mut clauses: Vec<String> = Vec::new();
        let mut binds: Vec<DynBind> = Vec::new();
        if let Some(a) = actor {
            clauses.push("actor = ?".to_string());
            binds.push(DynBind::Text(a.to_string()));
        }
        if let Some(a) = action {
            clauses.push("action = ?".to_string());
            binds.push(DynBind::Text(a.to_string()));
        }
        if let Some(t) = since {
            clauses.push("ts >= ?".to_string());
            binds.push(DynBind::Float(t));
        }
        if let Some(t) = until {
            clauses.push("ts <= ?".to_string());
            binds.push(DynBind::Float(t));
        }
        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };

        let select_sql = format!(
            "SELECT id, ts, actor, action, details
             FROM {}
             {where_sql}
             ORDER BY ts DESC, id DESC
             LIMIT ? OFFSET ?",
            self.q("audit")
        );
        type AuditSqliteRow = (String, f64, Option<String>, String, String);
        let mut q = sqlx::query_as::<_, AuditSqliteRow>(&select_sql);
        for b in &binds {
            q = match b {
                DynBind::Text(s) => q.bind(s),
                DynBind::Float(f) => q.bind(*f),
            };
        }
        let rows: Vec<AuditSqliteRow> = q
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .context("list engine.audit page")?;

        let count_sql = format!("SELECT COUNT(*) FROM {} {where_sql}", self.q("audit"));
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
                id,
                ts,
                actor,
                action,
                details: serde_json::from_str(&details).unwrap_or(serde_json::Value::Null),
            })
            .collect();
        Ok((items, total.0))
    }

    /// List currently-registered engine instances ordered by most-recent
    /// heartbeat first. SQLite is single-instance so this typically
    /// returns one row, but the table exists for parity with PG (the
    /// dashboard renders "1 instance" the same way it would render N).
    pub async fn list_instances(&self) -> Result<Vec<InstanceRecord>> {
        type InstanceSqliteRow = (String, f64, f64, String, Option<String>);
        let sql = format!(
            "SELECT id, started_at, last_heartbeat, namespaces, version
             FROM {}
             ORDER BY last_heartbeat DESC",
            self.q("instances")
        );
        let rows: Vec<InstanceSqliteRow> = sqlx::query_as(&sql)
            .fetch_all(&self.pool)
            .await
            .context("list engine.instances")?;
        Ok(rows
            .into_iter()
            .map(|(id, started_at, last_heartbeat, namespaces, version)| InstanceRecord {
                id,
                started_at,
                last_heartbeat,
                namespaces: serde_json::from_str(&namespaces).unwrap_or_default(),
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
        let sql = format!(
            "UPDATE {}
             SET enabled = ?,
                 enabled_at = CAST(strftime('%s','now') AS REAL),
                 enabled_by = COALESCE(?, enabled_by)
             WHERE name = ?",
            self.q("modules")
        );
        let res = sqlx::query(&sql)
            .bind(if enabled { 1i64 } else { 0i64 })
            .bind(actor)
            .bind(name)
            .execute(&self.pool)
            .await
            .context("set engine.modules.enabled")?;
        Ok(res.rows_affected() > 0)
    }
}

/// Dynamic-bind helper for [`SqliteEngineSchema::list_audit`]. The
/// filter set is small and stable; an enum keeps the bind ordering
/// trivial without pulling in a query builder.
enum DynBind {
    Text(String),
    Float(f64),
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
