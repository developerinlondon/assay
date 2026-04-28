#![cfg(all(test, feature = "backend-postgres"))]

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::OnceCell;

use super::*;
use crate::events::{EngineEventBus, EventFilter, NewEvent, PruneOpts, Subsystem};

static NS_COUNTER: AtomicU64 = AtomicU64::new(0);
static SCHEMA_READY: OnceCell<()> = OnceCell::const_new();

/// Read the shared PG test URL. Falls back to `ASSAY_PG_TEST_URL` if
/// `TEST_DATABASE_URL` isn't set; skip the test if neither is.
fn test_db_url() -> Option<String> {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("ASSAY_PG_TEST_URL"))
        .ok()
}

/// Unique namespace per test so parallel runs don't step on each other
/// sharing the same PG instance.
fn unique_ns(prefix: &str) -> String {
    let n = NS_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    format!("{prefix}_{pid}_{n}")
}

/// Prepare the `engine.events` table once per process. Concurrent
/// `CREATE TABLE IF NOT EXISTS` races the PG catalog (it inserts into
/// pg_class/pg_type before checking), so wrap the DDL in a `OnceCell`.
async fn prepare_pool() -> PgPool {
    let url = test_db_url().expect("TEST_DATABASE_URL not set");
    let pool = PgPool::connect(&url).await.unwrap();
    SCHEMA_READY
        .get_or_init(|| async {
            // PG's `CREATE SCHEMA IF NOT EXISTS` is documented as
            // idempotent but its implementation inserts into pg_namespace
            // *then* checks for the conflict — so two backends running it
            // at the same moment can both make it past the existence
            // probe and one then trips the unique index on
            // pg_namespace.nspname (SQLSTATE 23505). The OnceCell above
            // serialises this within one test binary, but multiple
            // crates' tests share the CI postgres container and race
            // each other across processes. Tolerate the duplicate-key
            // path: if the schema is already there, that's exactly the
            // post-condition we wanted.
            if let Err(e) = sqlx::query("CREATE SCHEMA IF NOT EXISTS engine")
                .execute(&pool)
                .await
            {
                let is_dup = e
                    .as_database_error()
                    .map(|d| d.code().as_deref() == Some("23505"))
                    .unwrap_or(false);
                if !is_dup {
                    panic!("create schema engine: {e}");
                }
            }
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS engine.events (
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
            sqlx::query(
                "CREATE INDEX IF NOT EXISTS idx_engine_events_ns_id ON engine.events(namespace, id)",
            )
            .execute(&pool)
            .await
            .unwrap();
        })
        .await;
    pool
}

#[tokio::test(flavor = "multi_thread")]
async fn append_then_read_round_trip() {
    let Some(url) = test_db_url() else {
        eprintln!("skipped: TEST_DATABASE_URL not set");
        return;
    };
    let ns = unique_ns("append");
    let pool = prepare_pool().await;
    let bus = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
    let id = bus
        .publish_committed(NewEvent {
            namespace: &ns,
            subsystem: Subsystem::Workflow,
            kind: "workflow_created",
            payload: serde_json::json!({ "workflow_id": "wf-1" }),
        })
        .await
        .unwrap();
    let evs = bus
        .read_since(&ns, None, &EventFilter::default(), 10)
        .await
        .unwrap();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].id, id);
    assert_eq!(evs[0].namespace, ns);
    assert_eq!(evs[0].kind, "workflow_created");
    assert_eq!(evs[0].payload, serde_json::json!({ "workflow_id": "wf-1" }));
}

#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
async fn cursor_replay_skips_earlier() {
    let Some(url) = test_db_url() else {
        eprintln!("skipped: TEST_DATABASE_URL not set");
        return;
    };
    let ns = unique_ns("cursor");
    let pool = prepare_pool().await;
    let bus = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
    let id1 = bus
        .publish_committed(NewEvent {
            namespace: &ns,
            subsystem: Subsystem::Workflow,
            kind: "a",
            payload: serde_json::json!({}),
        })
        .await
        .unwrap();
    let id2 = bus
        .publish_committed(NewEvent {
            namespace: &ns,
            subsystem: Subsystem::Workflow,
            kind: "b",
            payload: serde_json::json!({}),
        })
        .await
        .unwrap();
    let evs = bus
        .read_since(&ns, Some(id1), &EventFilter::default(), 10)
        .await
        .unwrap();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].id, id2);
}

#[tokio::test(flavor = "multi_thread")]
async fn subscribe_receives_local_publish() {
    let Some(url) = test_db_url() else {
        eprintln!("skipped: TEST_DATABASE_URL not set");
        return;
    };
    let ns = unique_ns("sublocal");
    let pool = prepare_pool().await;
    let bus = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
    let mut rx = bus.subscribe(&ns);
    bus.publish_committed(NewEvent {
        namespace: &ns,
        subsystem: Subsystem::Workflow,
        kind: "workflow_created",
        payload: serde_json::json!({}),
    })
    .await
    .unwrap();
    // Filter: the local broadcast is global (not namespaced) so drain
    // until we find our test's namespace or time out.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(!remaining.is_zero(), "timed out waiting for own publish");
        let ev = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("recv timed out")
            .unwrap();
        if ev.namespace == ns {
            assert_eq!(ev.kind, "workflow_created");
            break;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn subscribe_receives_cross_node_notify() {
    let Some(url) = test_db_url() else {
        eprintln!("skipped: TEST_DATABASE_URL not set");
        return;
    };
    let ns = unique_ns("cross");
    let pool = prepare_pool().await;
    let bus_a = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
    let bus_b = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
    let mut rx_b = bus_b.subscribe(&ns);
    // LISTEN bridge needs a moment to connect before cross-node NOTIFY
    // can be delivered.
    tokio::time::sleep(Duration::from_millis(500)).await;
    bus_a
        .publish_committed(NewEvent {
            namespace: &ns,
            subsystem: Subsystem::Workflow,
            kind: "x",
            payload: serde_json::json!({}),
        })
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(!remaining.is_zero(), "cross-node NOTIFY not received within 10s");
        let ev = tokio::time::timeout(remaining, rx_b.recv())
            .await
            .expect("recv timed out")
            .unwrap();
        if ev.namespace == ns {
            assert_eq!(ev.kind, "x");
            break;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn prune_removes_older_than_cutoff() {
    let Some(url) = test_db_url() else {
        eprintln!("skipped: TEST_DATABASE_URL not set");
        return;
    };
    let ns = unique_ns("prune");
    let pool = prepare_pool().await;
    let bus = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
    bus.publish_committed(NewEvent {
        namespace: &ns,
        subsystem: Subsystem::Workflow,
        kind: "x",
        payload: serde_json::json!({}),
    })
    .await
    .unwrap();
    let n = bus
        .prune_with(PruneOpts::new(f64::MAX).namespace(&ns))
        .await
        .unwrap();
    assert_eq!(n, 1, "prune_with should have removed exactly our row");
    let rest = bus
        .read_since(&ns, None, &EventFilter::default(), 10)
        .await
        .unwrap();
    assert!(rest.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn listener_still_works_across_bus_drop() {
    // Lightweight smoke test that two PgEngineEventBus instances
    // constructed against the same pool each get a working LISTEN
    // bridge (exercised by publish + subscribe after one is dropped).
    // Full server-side backend-termination would require
    // pg_terminate_backend on the listener's PID which is flaky in CI.
    //
    // TCP keepalive on PgConnectOptions isn't exposed in sqlx 0.8, so
    // we rely on OS-default keepalives + sqlx auto-reconnect. This
    // test just documents that dropping a bus doesn't poison future
    // listener setups on the same pool.
    let Some(url) = test_db_url() else {
        eprintln!("skipped: TEST_DATABASE_URL not set");
        return;
    };
    let ns = unique_ns("reconnect");
    let pool = prepare_pool().await;
    {
        let bus1 = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
        drop(bus1);
    }
    let bus2 = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
    let mut rx = bus2.subscribe(&ns);
    tokio::time::sleep(Duration::from_millis(500)).await;
    bus2.publish_committed(NewEvent {
        namespace: &ns,
        subsystem: Subsystem::Workflow,
        kind: "z",
        payload: serde_json::json!({}),
    })
    .await
    .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(!remaining.is_zero(), "reconnect smoke: event not received");
        let ev = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("recv timed out")
            .unwrap();
        if ev.namespace == ns {
            assert_eq!(ev.kind, "z");
            break;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
async fn cursor_before_oldest_returns_gone() {
    let Some(url) = test_db_url() else {
        eprintln!("skipped: TEST_DATABASE_URL not set");
        return;
    };
    let ns = unique_ns("gone");
    let pool = prepare_pool().await;
    let bus = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
    // Seed into namespace once, prune, seed again — namespace's oldest
    // id is now the second insert.
    bus.publish_committed(NewEvent {
        namespace: &ns,
        subsystem: Subsystem::Workflow,
        kind: "x",
        payload: serde_json::json!({}),
    })
    .await
    .unwrap();
    // Prune via namespace-scoped ts cutoff — global prune would fight
    // parallel tests.
    sqlx::query("DELETE FROM engine.events WHERE namespace = $1")
        .bind(&ns)
        .execute(&pool)
        .await
        .unwrap();
    let new_id = bus
        .publish_committed(NewEvent {
            namespace: &ns,
            subsystem: Subsystem::Workflow,
            kind: "y",
            payload: serde_json::json!({}),
        })
        .await
        .unwrap();
    let err = bus
        .read_since(&ns, Some(new_id - 100), &EventFilter::default(), 10)
        .await
        .unwrap_err();
    assert!(err.after < err.oldest);
}
