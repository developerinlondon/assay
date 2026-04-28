use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::SqlitePool;
use tokio::sync::broadcast;

use super::trait_::{
    CursorGoneError, EngineEventBus, Event, EventFilter, NewEvent, PruneOpts, Subsystem,
};

const LOCAL_BROADCAST_CAPACITY: usize = 1024;

pub struct SqliteEngineEventBus {
    pool: SqlitePool,
    local: broadcast::Sender<Arc<Event>>,
}

impl SqliteEngineEventBus {
    pub async fn new(pool: SqlitePool) -> Result<Self> {
        let (local, _) = broadcast::channel(LOCAL_BROADCAST_CAPACITY);
        Ok(Self { pool, local })
    }

    async fn oldest_id_inner(&self, namespace: &str) -> Result<Option<i64>> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM engine.events WHERE namespace = ?1 ORDER BY id ASC LIMIT 1",
        )
        .bind(namespace)
        .fetch_optional(&self.pool)
        .await
        .context("sqlite oldest_id")?;
        Ok(row.map(|r| r.0))
    }
}

#[async_trait]
impl EngineEventBus for SqliteEngineEventBus {
    async fn publish_committed(&self, ev: NewEvent<'_>) -> Result<i64> {
        let payload_str = serde_json::to_string(&ev.payload)?;
        let mut tx = self.pool.begin().await?;
        let row: (i64, f64) = sqlx::query_as(
            "INSERT INTO engine.events (namespace, subsystem, kind, payload)
             VALUES (?1, ?2, ?3, ?4)
             RETURNING id, ts",
        )
        .bind(ev.namespace)
        .bind(ev.subsystem.as_str())
        .bind(ev.kind)
        .bind(&payload_str)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        let event = Arc::new(Event {
            id: row.0,
            ts: row.1,
            namespace: ev.namespace.to_string(),
            subsystem: ev.subsystem,
            kind: ev.kind.to_string(),
            payload: ev.payload,
        });
        let _ = self.local.send(event);
        Ok(row.0)
    }

    async fn read_since(
        &self,
        namespace: &str,
        after: Option<i64>,
        filter: &EventFilter,
        limit: u32,
    ) -> std::result::Result<Vec<Event>, CursorGoneError> {
        if let Some(a) = after
            && let Ok(Some(oldest)) = self.oldest_id_inner(namespace).await
            && a < oldest - 1
        {
            return Err(CursorGoneError { after: a, oldest });
        }
        let rows: Vec<(i64, f64, String, String, String, String)> = sqlx::query_as(
            "SELECT id, ts, namespace, subsystem, kind, payload
             FROM engine.events
             WHERE namespace = ?1 AND (?2 IS NULL OR id > ?2)
             ORDER BY id ASC
             LIMIT ?3",
        )
        .bind(namespace)
        .bind(after)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!(?e, "sqlite read_since error");
            CursorGoneError {
                after: after.unwrap_or(0),
                oldest: 0,
            }
        })?;
        Ok(rows
            .into_iter()
            .map(|(id, ts, ns, subsystem, kind, payload_str)| {
                let payload: serde_json::Value =
                    serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null);
                Event {
                    id,
                    ts,
                    namespace: ns,
                    subsystem: Subsystem::from_str(&subsystem),
                    kind,
                    payload,
                }
            })
            .filter(|e| filter.matches(e))
            .collect())
    }

    fn subscribe(&self, _namespace: &str) -> broadcast::Receiver<Arc<Event>> {
        // SQLite is single-process; local broadcast is the full surface.
        // Namespace filter happens in read_since / SSE layer filter.
        self.local.subscribe()
    }

    async fn prune(&self, before_ts: f64) -> Result<u64> {
        let n = sqlx::query("DELETE FROM engine.events WHERE ts < ?1")
            .bind(before_ts)
            .execute(&self.pool)
            .await
            .context("sqlite prune (global)")?
            .rows_affected();
        Ok(n)
    }

    async fn prune_with(&self, opts: PruneOpts) -> Result<u64> {
        let n = match &opts.namespace {
            Some(ns) => sqlx::query(
                "DELETE FROM engine.events WHERE namespace = ?1 AND ts < ?2",
            )
            .bind(ns)
            .bind(opts.before_ts)
            .execute(&self.pool)
            .await
            .context("sqlite prune_with (scoped)")?
            .rows_affected(),
            None => sqlx::query("DELETE FROM engine.events WHERE ts < ?1")
                .bind(opts.before_ts)
                .execute(&self.pool)
                .await
                .context("sqlite prune_with (global)")?
                .rows_affected(),
        };
        Ok(n)
    }

    async fn oldest_id(&self, namespace: &str) -> Result<Option<i64>> {
        self.oldest_id_inner(namespace).await
    }
}

#[cfg(test)]
#[path = "sqlite_test.rs"]
mod sqlite_test;
