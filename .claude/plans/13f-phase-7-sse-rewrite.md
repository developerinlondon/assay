# Plan 13f — Phase 7: SSE Rewrite (`Last-Event-ID`, 410, Lagged Force-Close)

> Parent plan: [13-v0.13.1-engine-events-outbox.md](13-v0.13.1-engine-events-outbox.md) Prev:
> [13e-phase-6-delete-triggers-rewire.md](13e-phase-6-delete-triggers-rewire.md) — Next:
> [13g-phase-8-9-10-cleanup-ship.md](13g-phase-8-9-10-cleanup-ship.md)

---

## Phase 7 — SSE rewrite: filters, `Last-Event-ID`, 410 Gone, Lagged force-close

**Files:**

- Rewrite: `crates/assay-workflow/src/api/events.rs`
- Create: `crates/assay-workflow/tests/sse_replay.rs`
- Possibly modify: `crates/assay-workflow/tests/common/harness.rs` (expose an HTTP-server spawn
  helper if not already there)

### Endpoint contract

```
GET /api/v1/events/stream
  ?ns=<namespace>                        default "main"
  ?subsystem=workflow                    repeatable; empty = all
  ?workflow_id=<id>                      optional; filters by payload.workflow_id
  ?kind=workflow_status_changed          repeatable; empty = all

  Headers:
    Last-Event-ID: <i64>                 optional; cursor for replay

  Responses:
    200 text/event-stream
      id: <i64>
      event: <kind>
      data: {"id":..,"ts":..,"namespace":..,"subsystem":..,"kind":..,"payload":..}
    410 Gone                             Last-Event-ID older than retention
    503 Service Unavailable              bus not configured (tests only)
```

- [ ] **Step 7.1: Rewrite `api/events.rs`**

Replace the file contents with:

```rust
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::stream::{self, Stream, StreamExt};
use serde::Deserialize;
use tokio_stream::wrappers::BroadcastStream;

use assay_domain::events::{EventFilter, Subsystem};

use crate::ctx::WorkflowCtx;
use crate::store::WorkflowStore;

const REPLAY_PAGE_LIMIT: u32 = 500;

#[derive(Deserialize)]
struct StreamQuery {
    #[serde(default)]
    ns: Option<String>,
    #[serde(default)]
    subsystem: Option<Vec<String>>,
    #[serde(default)]
    workflow_id: Option<String>,
    #[serde(default)]
    kind: Option<Vec<String>>,
}

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<WorkflowCtx<S>>> {
    Router::new().route("/events/stream", get(event_stream))
}

async fn event_stream<S: WorkflowStore>(
    State(state): State<Arc<WorkflowCtx<S>>>,
    Query(q): Query<StreamQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(wf_bus) = state.bus() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "event bus not configured").into_response();
    };

    let namespace = q.ns.clone().unwrap_or_else(|| "main".to_string());
    let filter = EventFilter {
        subsystems: q
            .subsystem
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|s| match s.as_str() {
                "workflow" => Some(Subsystem::Workflow),
                "auth" => Some(Subsystem::Auth),
                "secrets" => Some(Subsystem::Secrets),
                "system" => Some(Subsystem::System),
                _ => None,
            })
            .collect(),
        kinds: q.kind.clone().unwrap_or_default(),
        workflow_id: q.workflow_id.clone(),
    };

    // EventSource spec: browser sends `Last-Event-ID` on reconnect.
    let last_id: Option<i64> = headers
        .get("last-event-id")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse().ok());

    let inner = wf_bus.inner();

    // Replay phase
    let replay_events = match inner
        .read_since(&namespace, last_id, &filter, REPLAY_PAGE_LIMIT)
        .await
    {
        Ok(evs) => evs,
        Err(gone) => {
            return (
                StatusCode::GONE,
                format!(
                    "cursor {} older than retention (oldest {}); resync via point queries then reconnect without Last-Event-ID",
                    gone.after, gone.oldest
                ),
            )
                .into_response();
        }
    };

    let replay_stream = stream::iter(
        replay_events
            .into_iter()
            .map(|e| Ok::<_, Infallible>(event_to_sse(&e))),
    );

    // Live phase
    let rx = inner.subscribe(&namespace);
    let ns_for_live = namespace.clone();
    let filter_for_live = filter.clone();
    let live_stream = BroadcastStream::new(rx).filter_map(move |result| {
        let ns = ns_for_live.clone();
        let f = filter_for_live.clone();
        async move {
            match result {
                Ok(arc_ev) => {
                    let ev = (*arc_ev).clone();
                    if ev.namespace != ns {
                        return None;
                    }
                    if !f.matches(&ev) {
                        return None;
                    }
                    Some(Ok::<_, Infallible>(event_to_sse(&ev)))
                }
                Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
                    // Force-close the stream so the client reconnects
                    // with Last-Event-ID and replays the gap via the
                    // engine_events table.
                    tracing::warn!(lagged = n, "SSE client lagged; forcing close");
                    None
                }
            }
        }
    });

    let combined = replay_stream.chain(live_stream);
    Sse::new(combined)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

fn event_to_sse(e: &assay_domain::events::Event) -> SseEvent {
    let data = serde_json::json!({
        "id": e.id,
        "ts": e.ts,
        "namespace": e.namespace,
        "subsystem": e.subsystem,
        "kind": e.kind,
        "payload": e.payload,
    })
    .to_string();
    SseEvent::default()
        .id(e.id.to_string())
        .event(&e.kind)
        .data(data)
}
```

- [ ] **Step 7.2: Write the integration tests**

The HTTP-server scaffolding (bind ephemeral port, spawn `axum::serve`, drive via `reqwest`) follows
the `assay-engine/tests/engine_smoke.rs` pattern. Lift the boilerplate into a per-test helper.

Create `crates/assay-workflow/tests/sse_replay.rs`:

```rust
#![cfg(feature = "backend-sqlite")]

use std::sync::Arc;
use std::time::Duration;

use assay_domain::events::{EngineEventBus, NewEvent, SqliteEngineEventBus, Subsystem};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

/// Spawn a minimal axum server with the SSE router and an in-memory
/// SQLite-backed bus. Returns (url, bus) so the test can publish events
/// directly to the bus and subscribe via HTTP.
async fn spawn_sse_server() -> (String, Arc<dyn EngineEventBus>) {
    let opts = SqliteConnectOptions::new()
        .filename(":memory:")
        .create_if_missing(true);
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
    let bus: Arc<dyn EngineEventBus> =
        Arc::new(SqliteEngineEventBus::new(pool.clone()).await.unwrap());

    // Build just the SSE router directly over the bus — the rest of
    // the WorkflowCtx surface is not exercised here. We stand up a
    // minimal Arc<WorkflowCtx<SqliteStore>> via the store + bus.
    use assay_workflow::ctx::WorkflowCtx;
    use assay_workflow::events::WorkflowEventBus;
    use assay_workflow::store::SqliteStore;
    let store = Arc::new(SqliteStore::from_pool(pool.clone()).await.unwrap());
    let wf_bus = WorkflowEventBus::new(Arc::clone(&bus));
    let ctx = WorkflowCtx::start(store).with_event_bus(wf_bus);
    let state = Arc::new(ctx);

    use axum::Router;
    let app: Router = assay_workflow::api::events::router().with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), bus)
}

async fn read_sse_ids(
    url: &str,
    last_event_id: Option<i64>,
    max: usize,
    timeout: Duration,
) -> Vec<i64> {
    let client = reqwest::Client::new();
    let mut req = client.get(format!("{url}/events/stream"));
    if let Some(id) = last_event_id {
        req = req.header("Last-Event-ID", id.to_string());
    }
    let resp = req.send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let mut stream = resp.bytes_stream();
    use futures_util::StreamExt;
    let mut buf = String::new();
    let mut ids = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    while ids.len() < max {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                buf.push_str(&String::from_utf8_lossy(&chunk));
                // Parse SSE frames: each event is separated by "\n\n";
                // each frame contains "id: N\n" lines.
                while let Some(sep) = buf.find("\n\n") {
                    let frame = buf[..sep].to_string();
                    buf = buf[sep + 2..].to_string();
                    for line in frame.lines() {
                        if let Some(v) = line.strip_prefix("id: ") {
                            if let Ok(id) = v.trim().parse::<i64>() {
                                ids.push(id);
                                if ids.len() >= max {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            _ => break,
        }
    }
    ids
}

#[tokio::test(flavor = "multi_thread")]
async fn sse_replay_from_last_event_id() {
    let (url, bus) = spawn_sse_server().await;

    // Publish three events.
    let id1 = bus
        .publish_committed(NewEvent {
            namespace: "main",
            subsystem: Subsystem::Workflow,
            kind: "workflow_created",
            payload: serde_json::json!({"workflow_id": "wf-1"}),
        })
        .await
        .unwrap();
    let _id2 = bus
        .publish_committed(NewEvent {
            namespace: "main",
            subsystem: Subsystem::Workflow,
            kind: "workflow_started",
            payload: serde_json::json!({"workflow_id": "wf-1"}),
        })
        .await
        .unwrap();
    let _id3 = bus
        .publish_committed(NewEvent {
            namespace: "main",
            subsystem: Subsystem::Workflow,
            kind: "workflow_completed",
            payload: serde_json::json!({"workflow_id": "wf-1"}),
        })
        .await
        .unwrap();

    // Subscribe with Last-Event-ID pointing at id1 — expect id2 and id3.
    let ids = read_sse_ids(&url, Some(id1), 2, Duration::from_secs(3)).await;
    assert_eq!(ids.len(), 2);
    assert!(ids.iter().all(|&i| i > id1));
}

#[tokio::test(flavor = "multi_thread")]
async fn sse_returns_410_when_cursor_before_retention() {
    let (url, bus) = spawn_sse_server().await;

    // Seed one row and prune everything.
    bus.publish_committed(NewEvent {
        namespace: "main",
        subsystem: Subsystem::Workflow,
        kind: "x",
        payload: serde_json::json!({}),
    })
    .await
    .unwrap();
    bus.prune(f64::MAX).await.unwrap();

    // Seed again so oldest_id is well above 0.
    let new_id = bus
        .publish_committed(NewEvent {
            namespace: "main",
            subsystem: Subsystem::Workflow,
            kind: "y",
            payload: serde_json::json!({}),
        })
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{url}/events/stream"))
        .header("Last-Event-ID", (new_id - 100).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 410);
}
```

> **Note on the test scaffolding:** if `crates/assay-workflow/src/api/events.rs::router` has
> visibility issues when called from tests, expose a `pub fn router_for_tests<S>()` helper or make
> the existing `router` function public. Keep the production entry-point unchanged.

- [ ] **Step 7.3: Verify + commit**

```bash
cargo test -p assay-workflow --features backend-sqlite --test sse_replay
```

Expected: both tests PASS.

Also re-run the full workspace suite to confirm no regression:

```bash
cargo test --workspace --lib --tests 2>&1 | tail -20
cargo test --test engine_smoke 2>&1 | tail -10
```

Expected: all green.

Commit:

```bash
git add crates/assay-workflow/src/api/events.rs crates/assay-workflow/tests/sse_replay.rs
git commit -m "$(cat <<'EOF'
feat(workflow/api): SSE with Last-Event-ID replay, 410 Gone, Lagged force-close

Endpoint surface:
  GET /api/v1/events/stream?ns=&subsystem=[..]&workflow_id=&kind=[..]
  Header: Last-Event-ID: <bigint>

Behaviour:
  1. Replay phase reads engine_events since Last-Event-ID (or from
     beginning if absent) and emits matching events as SSE frames.
  2. If Last-Event-ID is older than the namespace's oldest retained
     id, return HTTP 410 Gone with a body instructing the client to
     snapshot + resync.
  3. Live phase subscribes to the node-local broadcast; emits frames
     matching the filter.
  4. broadcast::Lagged closes the stream; client reconnects with the
     last id it received and replays the gap.

Tests:
  sse_replay_from_last_event_id        — id1 cursor → id2, id3 arrive
  sse_returns_410_when_cursor_before_retention
EOF
)"
```

---

## Exit criteria for Phase 7

```bash
cargo test -p assay-workflow --features backend-sqlite --test sse_replay   # 2 PASS
cargo test --workspace --lib --tests                                        # all pass
cargo test --test engine_smoke                                              # pass (new SSE shape)
git log --oneline -6                                                        # phases 1-7 commits
```

Move on to [13g-phase-8-9-10-cleanup-ship.md](13g-phase-8-9-10-cleanup-ship.md).
