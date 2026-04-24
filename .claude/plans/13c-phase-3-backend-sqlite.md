# Plan 13c — Phase 3: SQLite Backend (`SqliteEngineEventBus`)

> Parent plan: [13-v0.13.1-engine-events-outbox.md](13-v0.13.1-engine-events-outbox.md) Prev:
> [13b-phase-2-backend-pg.md](13b-phase-2-backend-pg.md) — Next:
> [13d-phase-4-5-typed-wrapper-cutover.md](13d-phase-4-5-typed-wrapper-cutover.md)

---

## Phase 3 — SQLite: `engine_events` schema + `SqliteEngineEventBus`

SQLite is single-process by decision #13 (Q1 answer B). No LISTEN equivalent; same
`tokio::broadcast` path as PG but without the cross-process bridge.

**Files:**

- Modify: `crates/assay-workflow/src/store/sqlite.rs` (SCHEMA block)
- Create: `crates/assay-domain/src/events/sqlite.rs`
- Create: `crates/assay-domain/src/events/sqlite_test.rs`
- Modify: `crates/assay-domain/src/events/mod.rs`

- [ ] **Step 3.1: Add `engine_events` DDL to SQLite SCHEMA**

In `crates/assay-workflow/src/store/sqlite.rs`, append to the SCHEMA const:

```sql
CREATE TABLE IF NOT EXISTS engine_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    ts              REAL NOT NULL DEFAULT (CAST(strftime('%s','now') AS REAL)),
    namespace       TEXT NOT NULL,
    subsystem       TEXT NOT NULL,
    kind            TEXT NOT NULL,
    payload         TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_engine_events_ns_id ON engine_events(namespace, id);
CREATE INDEX IF NOT EXISTS idx_engine_events_ts_prune ON engine_events(ts);
```

Run:

```bash
cargo check -p assay-workflow --features backend-sqlite
```

Expected: PASS.

- [ ] **Step 3.2: Write SQLite tests first (TDD)**

Create `crates/assay-domain/src/events/sqlite_test.rs`:

```rust
#![cfg(all(test, feature = "backend-sqlite"))]

use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

use super::*;
use crate::events::{EngineEventBus, EventFilter, NewEvent, Subsystem};

async fn fresh_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::new()
        .filename(":memory:")
        .create_if_missing(true);
    // :memory: is per-connection; cap the pool at 1 so everyone shares
    // the same DB instance.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS engine_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts REAL NOT NULL DEFAULT (CAST(strftime('%s','now') AS REAL)),
            namespace TEXT NOT NULL,
            subsystem TEXT NOT NULL,
            kind TEXT NOT NULL,
            payload TEXT NOT NULL DEFAULT '{}')",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_append_then_read() {
    let pool = fresh_pool().await;
    let bus = SqliteEngineEventBus::new(pool.clone()).await.unwrap();
    let id = bus
        .publish_committed(NewEvent {
            namespace: "main",
            subsystem: Subsystem::Workflow,
            kind: "x",
            payload: serde_json::json!({"k": "v"}),
        })
        .await
        .unwrap();
    let evs = bus
        .read_since("main", None, &EventFilter::default(), 10)
        .await
        .unwrap();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].id, id);
    assert_eq!(evs[0].payload, serde_json::json!({"k": "v"}));
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_subscribe_same_process() {
    let pool = fresh_pool().await;
    let bus = SqliteEngineEventBus::new(pool.clone()).await.unwrap();
    let mut rx = bus.subscribe("main");
    bus.publish_committed(NewEvent {
        namespace: "main",
        subsystem: Subsystem::Workflow,
        kind: "hi",
        payload: serde_json::json!({}),
    })
    .await
    .unwrap();
    let ev = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ev.kind, "hi");
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_prune_idempotent() {
    let pool = fresh_pool().await;
    let bus = SqliteEngineEventBus::new(pool.clone()).await.unwrap();
    bus.publish_committed(NewEvent {
        namespace: "main",
        subsystem: Subsystem::Workflow,
        kind: "x",
        payload: serde_json::json!({}),
    })
    .await
    .unwrap();
    let n = bus.prune(f64::MAX).await.unwrap();
    assert_eq!(n, 1);
    let n2 = bus.prune(f64::MAX).await.unwrap();
    assert_eq!(n2, 0, "prune must be idempotent");
}
```

Run:

```bash
cargo test -p assay-domain --features backend-sqlite --lib -- sqlite_test
```

Expected: all 3 tests FAIL with "SqliteEngineEventBus not found". Correct red state.

- [ ] **Step 3.3: Implement `SqliteEngineEventBus`**

Create `crates/assay-domain/src/events/sqlite.rs`:

```rust
#![cfg(feature = "backend-sqlite")]

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::SqlitePool;
use tokio::sync::broadcast;

use super::trait_::{
    CursorGoneError, EngineEventBus, Event, EventFilter, NewEvent, Subsystem,
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
            "SELECT id FROM engine_events WHERE namespace = ?1 ORDER BY id ASC LIMIT 1",
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
            "INSERT INTO engine_events (namespace, subsystem, kind, payload)
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
        if let Some(a) = after {
            if let Ok(Some(oldest)) = self.oldest_id_inner(namespace).await {
                if a < oldest - 1 {
                    return Err(CursorGoneError { after: a, oldest });
                }
            }
        }
        let rows: Vec<(i64, f64, String, String, String, String)> = sqlx::query_as(
            "SELECT id, ts, namespace, subsystem, kind, payload
             FROM engine_events
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
        let n = sqlx::query("DELETE FROM engine_events WHERE ts < ?1")
            .bind(before_ts)
            .execute(&self.pool)
            .await
            .context("sqlite prune")?
            .rows_affected();
        Ok(n)
    }

    async fn oldest_id(&self, namespace: &str) -> Result<Option<i64>> {
        self.oldest_id_inner(namespace).await
    }
}

#[cfg(test)]
mod sqlite_test;
```

- [ ] **Step 3.4: Register module + pub-use**

Add to `crates/assay-domain/src/events/mod.rs`:

```rust
#[cfg(feature = "backend-sqlite")]
pub mod sqlite;

#[cfg(feature = "backend-sqlite")]
pub use sqlite::SqliteEngineEventBus;
```

- [ ] **Step 3.5: Verify + commit**

```bash
cargo test -p assay-domain --features backend-sqlite --lib -- sqlite_test
```

Expected: all 3 tests PASS.

Also verify the feature matrix still builds with both features on:

```bash
cargo check -p assay-domain --features backend-postgres,backend-sqlite
```

Expected: PASS.

Commit:

```bash
git add crates/assay-domain/ crates/assay-workflow/src/store/sqlite.rs
git commit -m "$(cat <<'EOF'
feat(domain/events/sqlite): SqliteEngineEventBus impl

Same engine_events table shape as PG; single-process by design (no
LISTEN equivalent). Local tokio::broadcast is the only fan-out.
Multi-node SQLite is not supported per decision #13.

Tests green: append-read round-trip, subscribe, prune idempotency.
EOF
)"
```

---

## Exit criteria for Phase 3

```bash
cargo test -p assay-domain --features backend-sqlite --lib -- sqlite_test   # 3 PASS
cargo test -p assay-domain --features backend-postgres --lib -- pg_test     # 6 PASS
cargo check -p assay-domain --features backend-postgres,backend-sqlite      # clean
```

Move on to [13d-phase-4-5-typed-wrapper-cutover.md](13d-phase-4-5-typed-wrapper-cutover.md).
