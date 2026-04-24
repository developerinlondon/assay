#![cfg(feature = "backend-postgres")]

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::postgres::{PgConnectOptions, PgListener, PgPool, PgPoolOptions};
use tokio::sync::broadcast;

use super::trait_::{CursorGoneError, EngineEventBus, Event, EventFilter, NewEvent, Subsystem};

/// Default capacity of the node-local broadcast channel. A slow SSE
/// client that lags past this will be force-closed and reconnects
/// with cursor replay.
const LOCAL_BROADCAST_CAPACITY: usize = 1024;

/// Channel name used for `LISTEN`/`NOTIFY`. One channel per namespace.
fn channel_name(ns: &str) -> String {
    format!("assay_engine_events_{ns}")
}

pub struct PgEngineEventBus {
    pool: PgPool,
    local: broadcast::Sender<Arc<Event>>,
    /// Dedicated 1-conn pool with TCP keepalives (30s/10s/3 retries) for
    /// the PgListener connection. Kernel detects silently-dead TCP in
    /// ~60s so reconnect + cursor replay covers the gap.
    listener_pool: PgPool,
}

impl PgEngineEventBus {
    /// Construct a new bus over an existing pool. `db_url` is used to
    /// build a dedicated 1-connection pool for the `PgListener` so
    /// the LISTEN connection is isolated from application traffic.
    /// `PgListener` auto-reconnects on recv errors.
    ///
    /// TCP keepalive is OS-default here (sqlx 0.8 doesn't expose
    /// `keepalives*` on `PgConnectOptions`). Silently-dead TCP still
    /// surfaces on the next `recv()` error; the reconnect loop +
    /// cursor replay cover the gap. Kernel defaults on Linux are
    /// ~2h idle which is looser than ideal but acceptable given
    /// the durable outbox makes replay cheap.
    pub async fn new(pool: PgPool, db_url: &str) -> Result<Self> {
        let listener_opts =
            PgConnectOptions::from_str(db_url).context("invalid db_url for listener")?;
        let listener_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(listener_opts)
            .await
            .context("listener pool connect")?;
        let (local, _) = broadcast::channel(LOCAL_BROADCAST_CAPACITY);
        Ok(Self {
            pool,
            local,
            listener_pool,
        })
    }

    /// Spawn the LISTEN bridge for `namespace` if one isn't already
    /// running. Safe to call repeatedly; de-duplicates by channel name.
    fn ensure_listener(&self, namespace: &str) {
        static REGISTRY: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
        let reg = REGISTRY.get_or_init(|| Mutex::new(HashSet::new()));
        let ch = channel_name(namespace);
        {
            let mut g = reg.lock().unwrap();
            if !g.insert(ch.clone()) {
                return;
            }
        }
        let pool = self.pool.clone();
        let listener_pool = self.listener_pool.clone();
        let local = self.local.clone();
        tokio::spawn(async move {
            loop {
                match PgListener::connect_with(&listener_pool).await {
                    Ok(mut listener) => {
                        if let Err(e) = listener.listen(&ch).await {
                            tracing::warn!(?e, channel = %ch, "LISTEN failed; retrying in 2s");
                            tokio::time::sleep(Duration::from_secs(2)).await;
                            continue;
                        }
                        tracing::info!(channel = %ch, "engine_events LISTEN bridge online");
                        loop {
                            match listener.recv().await {
                                Ok(n) => {
                                    let id: i64 = match n.payload().parse() {
                                        Ok(v) => v,
                                        Err(e) => {
                                            tracing::warn!(?e, payload = n.payload(), "bad NOTIFY payload");
                                            continue;
                                        }
                                    };
                                    match sqlx::query_as::<_, PgEventRow>(
                                        "SELECT id, ts, namespace, subsystem, kind, payload
                                         FROM engine_events WHERE id = $1",
                                    )
                                    .bind(id)
                                    .fetch_optional(&pool)
                                    .await
                                    {
                                        Ok(Some(row)) => {
                                            let _ = local.send(Arc::new(row.into_event()));
                                        }
                                        Ok(None) => {
                                            tracing::debug!(id, "NOTIFY id not in table (pruned?)");
                                        }
                                        Err(e) => {
                                            tracing::warn!(?e, id, "engine_events SELECT after NOTIFY failed");
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(?e, channel = %ch, "LISTEN recv error; reconnecting");
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(?e, "PgListener connect failed; retrying in 5s");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        });
    }

    async fn oldest_id_inner(&self, namespace: &str) -> Result<Option<i64>> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM engine_events WHERE namespace = $1 ORDER BY id ASC LIMIT 1",
        )
        .bind(namespace)
        .fetch_optional(&self.pool)
        .await
        .context("oldest_id query")?;
        Ok(row.map(|r| r.0))
    }
}

#[derive(sqlx::FromRow)]
struct PgEventRow {
    id: i64,
    ts: f64,
    namespace: String,
    subsystem: String,
    kind: String,
    payload: serde_json::Value,
}

impl PgEventRow {
    fn into_event(self) -> Event {
        let ss = Subsystem::from_str(&self.subsystem);
        Event {
            id: self.id,
            ts: self.ts,
            namespace: self.namespace,
            subsystem: ss,
            kind: self.kind,
            payload: self.payload,
        }
    }
}

#[async_trait]
impl EngineEventBus for PgEngineEventBus {
    async fn publish_committed(&self, ev: NewEvent<'_>) -> Result<i64> {
        let mut tx = self.pool.begin().await?;
        let row: (i64, f64) = sqlx::query_as(
            "INSERT INTO engine_events (namespace, subsystem, kind, payload)
             VALUES ($1, $2, $3, $4)
             RETURNING id, ts",
        )
        .bind(ev.namespace)
        .bind(ev.subsystem.as_str())
        .bind(ev.kind)
        .bind(&ev.payload)
        .fetch_one(&mut *tx)
        .await?;
        let ch = channel_name(ev.namespace);
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(&ch)
            .bind(row.0.to_string())
            .execute(&mut *tx)
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
        if let Some(a) = after {
            if let Ok(Some(oldest)) = self.oldest_id_inner(namespace).await {
                if a < oldest - 1 {
                    return Err(CursorGoneError { after: a, oldest });
                }
            }
        }
        let rows: Vec<PgEventRow> = sqlx::query_as(
            "SELECT id, ts, namespace, subsystem, kind, payload
             FROM engine_events
             WHERE namespace = $1 AND ($2::bigint IS NULL OR id > $2)
             ORDER BY id ASC
             LIMIT $3",
        )
        .bind(namespace)
        .bind(after)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!(?e, "read_since DB error");
            CursorGoneError {
                after: after.unwrap_or(0),
                oldest: 0,
            }
        })?;
        Ok(rows
            .into_iter()
            .map(PgEventRow::into_event)
            .filter(|e| filter.matches(e))
            .collect())
    }

    fn subscribe(&self, namespace: &str) -> broadcast::Receiver<Arc<Event>> {
        self.ensure_listener(namespace);
        self.local.subscribe()
    }

    async fn prune(&self, before_ts: f64) -> Result<u64> {
        let n = sqlx::query("DELETE FROM engine_events WHERE ts < $1")
            .bind(before_ts)
            .execute(&self.pool)
            .await
            .context("engine_events prune")?
            .rows_affected();
        Ok(n)
    }

    async fn oldest_id(&self, namespace: &str) -> Result<Option<i64>> {
        self.oldest_id_inner(namespace).await
    }
}

#[cfg(test)]
#[path = "pg_test.rs"]
mod pg_test;
