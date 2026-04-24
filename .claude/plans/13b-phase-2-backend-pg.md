# Plan 13b — Phase 2: PG Backend (`PgEngineEventBus` + `engine_events` DDL)

> Parent plan: [13-v0.13.1-engine-events-outbox.md](13-v0.13.1-engine-events-outbox.md) Prev:
> [13a-phase-0-1-trait-scaffold.md](13a-phase-0-1-trait-scaffold.md) — Next:
> [13c-phase-3-backend-sqlite.md](13c-phase-3-backend-sqlite.md)

---

## Phase 2 — PG: `engine_events` schema + `PgEngineEventBus`

**Files:**

- Modify: `crates/assay-workflow/src/store/postgres.rs` (SCHEMA block only in this phase)
- Create: `crates/assay-domain/src/events/pg.rs`
- Create: `crates/assay-domain/src/events/pg_test.rs`
- Modify: `crates/assay-domain/Cargo.toml` (feature-gated sqlx)
- Modify: `crates/assay-domain/src/events/mod.rs` (feature-gate `pg`)

> **Crate boundary:** `assay-domain` gains `sqlx` as a feature-gated dep (`backend-postgres`).
> Downstream crates that compile only the trait (no impl) don't pay the build cost.

- [ ] **Step 2.1: Add feature-gated `sqlx` to `assay-domain/Cargo.toml`**

Add under `[features]`:

```toml
[features]
default = []
backend-postgres = ["dep:sqlx", "sqlx/postgres", "dep:async-stream"]
backend-sqlite = ["dep:sqlx", "sqlx/sqlite"]
```

Add under `[dependencies]`:

```toml
sqlx = { version = "0.8", features = ["runtime-tokio-rustls"], optional = true }
async-stream = { version = "0.3", optional = true }
```

Run:

```bash
cargo check -p assay-domain --features backend-postgres
```

Expected: PASS.

- [ ] **Step 2.2: Add `engine_events` DDL to PG SCHEMA block**

In `crates/assay-workflow/src/store/postgres.rs`, append to the SCHEMA const (before the closing
`"#;`):

```sql
CREATE TABLE IF NOT EXISTS engine_events (
    id              BIGSERIAL PRIMARY KEY,
    ts              DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
    namespace       TEXT NOT NULL,
    subsystem       TEXT NOT NULL,
    kind            TEXT NOT NULL,
    payload         JSONB NOT NULL DEFAULT '{}'::jsonb
);
CREATE INDEX IF NOT EXISTS idx_engine_events_ns_id ON engine_events(namespace, id);
CREATE INDEX IF NOT EXISTS idx_engine_events_ts_prune ON engine_events(ts);
```

Verify the `sanitise_schema` split still handles this — it does; `;` delimiters are outside any
dollar-quoted block.

Run:

```bash
cargo check -p assay-workflow --features backend-postgres
```

Expected: PASS.

- [ ] **Step 2.3: Write the PG impl tests FIRST (TDD)**

Create `crates/assay-domain/src/events/pg_test.rs`:

```rust
#![cfg(all(test, feature = "backend-postgres"))]

use std::time::Duration;

use sqlx::PgPool;

use super::*;
use crate::events::{EngineEventBus, EventFilter, NewEvent, Subsystem};

/// Prepares a clean schema against a testcontainer-issued URL.
/// Env var `ASSAY_PG_TEST_URL` must point to a running PG 18.
async fn fresh_pool() -> PgPool {
    let url = std::env::var("ASSAY_PG_TEST_URL")
        .expect("set ASSAY_PG_TEST_URL to run PG bus tests");
    let pool = PgPool::connect(&url).await.unwrap();

    // Apply just the engine_events DDL we rely on.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS engine_events (
            id BIGSERIAL PRIMARY KEY,
            ts DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
            namespace TEXT NOT NULL,
            subsystem TEXT NOT NULL,
            kind TEXT NOT NULL,
            payload JSONB NOT NULL DEFAULT '{}'::jsonb)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("TRUNCATE engine_events").execute(&pool).await.unwrap();
    pool
}

#[tokio::test(flavor = "multi_thread")]
async fn append_then_read_round_trip() {
    let pool = fresh_pool().await;
    let url = std::env::var("ASSAY_PG_TEST_URL").unwrap();
    let bus = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
    let id = bus
        .publish_committed(NewEvent {
            namespace: "main",
            subsystem: Subsystem::Workflow,
            kind: "workflow_created",
            payload: serde_json::json!({ "workflow_id": "wf-1" }),
        })
        .await
        .unwrap();
    let evs = bus
        .read_since("main", None, &EventFilter::default(), 10)
        .await
        .unwrap();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].id, id);
    assert_eq!(evs[0].namespace, "main");
    assert_eq!(evs[0].kind, "workflow_created");
    assert_eq!(
        evs[0].payload,
        serde_json::json!({ "workflow_id": "wf-1" })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cursor_replay_skips_earlier() {
    let pool = fresh_pool().await;
    let url = std::env::var("ASSAY_PG_TEST_URL").unwrap();
    let bus = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
    let id1 = bus
        .publish_committed(NewEvent {
            namespace: "main",
            subsystem: Subsystem::Workflow,
            kind: "a",
            payload: serde_json::json!({}),
        })
        .await
        .unwrap();
    let id2 = bus
        .publish_committed(NewEvent {
            namespace: "main",
            subsystem: Subsystem::Workflow,
            kind: "b",
            payload: serde_json::json!({}),
        })
        .await
        .unwrap();
    let evs = bus
        .read_since("main", Some(id1), &EventFilter::default(), 10)
        .await
        .unwrap();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].id, id2);
}

#[tokio::test(flavor = "multi_thread")]
async fn subscribe_receives_local_publish() {
    let pool = fresh_pool().await;
    let url = std::env::var("ASSAY_PG_TEST_URL").unwrap();
    let bus = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
    let mut rx = bus.subscribe("main");
    bus.publish_committed(NewEvent {
        namespace: "main",
        subsystem: Subsystem::Workflow,
        kind: "workflow_created",
        payload: serde_json::json!({}),
    })
    .await
    .unwrap();
    let ev = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout waiting for event")
        .unwrap();
    assert_eq!(ev.kind, "workflow_created");
}

#[tokio::test(flavor = "multi_thread")]
async fn subscribe_receives_cross_node_notify() {
    let pool = fresh_pool().await;
    let url = std::env::var("ASSAY_PG_TEST_URL").unwrap();
    let bus_a = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
    let bus_b = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
    let mut rx_b = bus_b.subscribe("main");
    // Give the LISTEN bridge a moment to connect before publishing.
    tokio::time::sleep(Duration::from_millis(200)).await;
    bus_a
        .publish_committed(NewEvent {
            namespace: "main",
            subsystem: Subsystem::Workflow,
            kind: "x",
            payload: serde_json::json!({}),
        })
        .await
        .unwrap();
    let ev = tokio::time::timeout(Duration::from_secs(5), rx_b.recv())
        .await
        .expect("NOTIFY cross-node failed")
        .unwrap();
    assert_eq!(ev.kind, "x");
}

#[tokio::test(flavor = "multi_thread")]
async fn prune_removes_older_than_cutoff() {
    let pool = fresh_pool().await;
    let url = std::env::var("ASSAY_PG_TEST_URL").unwrap();
    let bus = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
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
    let rest = bus
        .read_since("main", None, &EventFilter::default(), 10)
        .await
        .unwrap();
    assert!(rest.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn cursor_before_oldest_returns_gone() {
    let pool = fresh_pool().await;
    let url = std::env::var("ASSAY_PG_TEST_URL").unwrap();
    let bus = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
    // Seed one row then prune everything.
    bus.publish_committed(NewEvent {
        namespace: "main",
        subsystem: Subsystem::Workflow,
        kind: "x",
        payload: serde_json::json!({}),
    })
    .await
    .unwrap();
    bus.prune(f64::MAX).await.unwrap();
    // Seed again; now oldest_id is this new id.
    let new_id = bus
        .publish_committed(NewEvent {
            namespace: "main",
            subsystem: Subsystem::Workflow,
            kind: "y",
            payload: serde_json::json!({}),
        })
        .await
        .unwrap();
    // Request with a cursor 100 behind the oldest retained id.
    let err = bus
        .read_since("main", Some(new_id - 100), &EventFilter::default(), 10)
        .await
        .unwrap_err();
    assert!(err.after < err.oldest);
}
```

Run:

```bash
cargo test -p assay-domain --features backend-postgres --lib -- pg_test
```

Expected: all 6 tests FAIL with "PgEngineEventBus not found" / unresolved imports. That is the
correct red state for TDD.

- [ ] **Step 2.4: Implement `PgEngineEventBus`**

Create `crates/assay-domain/src/events/pg.rs`:

```rust
#![cfg(feature = "backend-postgres")]

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::postgres::{PgConnectOptions, PgListener, PgPool};
use tokio::sync::broadcast;

use super::trait_::{
    CursorGoneError, EngineEventBus, Event, EventFilter, NewEvent, Subsystem,
};

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
    listener_opts: PgConnectOptions,
}

impl PgEngineEventBus {
    /// Construct a new bus over an existing pool. `db_url` is used only
    /// for the dedicated `PgListener` connection where we want to set
    /// TCP keepalive options independently of the pool. `PgListener`
    /// auto-reconnects on recv errors; kernel keepalive (30s idle,
    /// 10s interval, 3 retries) detects silently-dead TCP within ~60s.
    pub async fn new(pool: PgPool, db_url: &str) -> Result<Self> {
        let listener_opts = PgConnectOptions::from_str(db_url)
            .context("invalid db_url for listener")?
            .keepalives(true)
            .keepalives_idle(Duration::from_secs(30))
            .keepalives_interval(Duration::from_secs(10))
            .keepalives_retries(3);
        let (local, _) = broadcast::channel(LOCAL_BROADCAST_CAPACITY);
        Ok(Self { pool, local, listener_opts })
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
                return; // already listening on this channel
            }
        }
        let pool = self.pool.clone();
        let listener_opts = self.listener_opts.clone();
        let local = self.local.clone();
        tokio::spawn(async move {
            loop {
                match PgListener::connect_with_options(&listener_opts).await {
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
        if !matches!(
            ss,
            Subsystem::Workflow | Subsystem::Auth | Subsystem::Secrets | Subsystem::System
        ) {
            tracing::warn!(subsystem = %self.subsystem, "unknown subsystem in engine_events row");
        }
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
        // pg_notify is transactional — fires at COMMIT, so the INSERT
        // and the NOTIFY are atomically published together.
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(&ch)
            .bind(row.0.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        // Same-node shortcut: broadcast immediately without waiting for
        // the LISTEN bridge round-trip.
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
        // Cursor-gone detection: the client's cursor is "gone" when it's
        // more than one id behind the oldest retained id. (Exactly one
        // behind is fine — it means "I saw everything up to oldest-1".)
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
mod pg_test;
```

- [ ] **Step 2.5: Register module + pub-use**

Replace the body of `crates/assay-domain/src/events/mod.rs` with:

```rust
//! Engine-wide CDC outbox. See `trait_.rs` for the trait surface and
//! `pg.rs` / `sqlite.rs` for the backend impls (feature-gated).

pub mod trait_;
pub use trait_::*;

#[cfg(feature = "backend-postgres")]
pub mod pg;

#[cfg(feature = "backend-postgres")]
pub use pg::PgEngineEventBus;
```

- [ ] **Step 2.6: Verify test suite**

Set `ASSAY_PG_TEST_URL` to a running PG 18 container (the existing
`crates/assay-workflow/tests/common/harness.rs` spawns one via testcontainers-modules; either lift
that pattern or run an ad-hoc container for this phase). Run:

```bash
cargo test -p assay-domain --features backend-postgres --lib -- pg_test
```

Expected: all 6 tests PASS.

- [ ] **Step 2.7: Commit**

```bash
git add crates/assay-domain/ crates/assay-workflow/src/store/postgres.rs
git commit -m "$(cat <<'EOF'
feat(domain/events/pg): PgEngineEventBus + engine_events PG schema

- engine_events table (BIGSERIAL id, namespace, subsystem, kind, jsonb
  payload) added to PG SCHEMA; indexed by (namespace,id) and (ts) for
  cursor reads and prune sweeps.
- PgEngineEventBus wraps a PgPool; publish_committed does INSERT +
  pg_notify inside one tx so the NOTIFY fires atomically at commit.
- Per-namespace PgListener bridge spawned lazily on subscribe();
  listener receives id, selects the row, fans into local broadcast.
- TCP keepalive (30s idle / 10s interval / 3 retries) on the listener
  connection + sqlx auto-reconnect cover the liveness case without
  application-level heartbeats.
- Cursor-gone detection via oldest_id lookup so SSE can return 410.

Tests (6, all green): round-trip, cursor replay, same-node subscribe,
cross-node NOTIFY, prune, cursor-before-oldest.
EOF
)"
```

---

## Exit criteria for Phase 2

```bash
cargo test -p assay-domain --features backend-postgres --lib -- pg_test   # 6 PASS
cargo check --workspace                                                    # clean
git log --oneline -2                                                       # shows both commits
```

Move on to [13c-phase-3-backend-sqlite.md](13c-phase-3-backend-sqlite.md).
