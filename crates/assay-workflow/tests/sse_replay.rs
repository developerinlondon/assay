//! Integration tests for the engine-events SSE endpoint.
//!
//! Spins up a minimal axum server with the `/events/stream` route backed
//! by an in-memory SQLite bus. Publishes events directly into the bus
//! and asserts what the HTTP client sees (replay via `Last-Event-ID`,
//! HTTP 410 on pre-retention cursors).

#![cfg(feature = "backend-sqlite")]

use std::sync::Arc;
use std::time::Duration;

use assay_domain::events::{
    EngineEventBus, NewEvent, SqliteEngineEventBus, Subsystem,
};
use assay_workflow::WorkflowCtx;
use assay_workflow::events::WorkflowEventBus;
use axum::Router;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// Wire a SqliteStore + SqliteEngineEventBus on the same in-memory pool,
/// stand up an axum server with just the events router, and return the
/// base URL + bus so the test can push events directly.
async fn spawn_sse_server() -> (String, Arc<dyn EngineEventBus>) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let suffix = format!(
        "{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let engine_alias =
        format!("file:assay_sse_engine_{suffix}?mode=memory&cache=shared");
    let workflow_alias =
        format!("file:assay_sse_workflow_{suffix}?mode=memory&cache=shared");

    let opts = SqliteConnectOptions::new()
        .filename(":memory:")
        .create_if_missing(true);
    let pool: SqlitePool = SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(move |conn, _meta| {
            let engine_alias = engine_alias.clone();
            let workflow_alias = workflow_alias.clone();
            Box::pin(async move {
                use sqlx::Executor;
                conn.execute(
                    format!("ATTACH DATABASE '{engine_alias}' AS engine").as_str(),
                )
                .await?;
                conn.execute(
                    format!("ATTACH DATABASE '{workflow_alias}' AS workflow").as_str(),
                )
                .await?;
                Ok(())
            })
        })
        .connect_with(opts)
        .await
        .unwrap();

    // Build the engine.events table + the rest of the schema the
    // SqliteStore migrates in. SqliteStore::from_attached_pool does both.
    let store = Arc::new(
        assay_workflow::SqliteStore::from_attached_pool(pool.clone())
            .await
            .unwrap(),
    );
    let bus: Arc<dyn EngineEventBus> =
        Arc::new(SqliteEngineEventBus::new(pool.clone()).await.unwrap());

    let wf_bus = WorkflowEventBus::new(Arc::clone(&bus));
    let ctx = WorkflowCtx::start(store).with_event_bus(wf_bus);
    let state = Arc::new(ctx);

    let app: Router = assay_workflow::api::events::router()
        .with_state(state);

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
    assert_eq!(resp.status(), 200, "SSE replay expected 200");
    use futures_util::StreamExt;
    let mut stream = Box::pin(resp.bytes_stream());
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
                while let Some(sep) = buf.find("\n\n") {
                    let frame = buf[..sep].to_string();
                    buf = buf[sep + 2..].to_string();
                    for line in frame.lines() {
                        if let Some(v) = line.strip_prefix("id: ")
                            && let Ok(id) = v.trim().parse::<i64>()
                        {
                            ids.push(id);
                            if ids.len() >= max {
                                break;
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
            payload: serde_json::json !({"workflow_id": "wf-1"}),
        })
        .await
        .unwrap();

    // Reconnect with cursor = id1 — expect id2, id3 from the replay.
    let ids = read_sse_ids(&url, Some(id1), 2, Duration::from_secs(3)).await;
    assert_eq!(ids.len(), 2, "expected 2 replay frames, got {ids:?}");
    assert!(ids.iter().all(|&i| i > id1), "all ids must be > cursor: {ids:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn sse_returns_410_when_cursor_before_retention() {
    let (url, bus) = spawn_sse_server().await;

    bus.publish_committed(NewEvent {
        namespace: "main",
        subsystem: Subsystem::Workflow,
        kind: "x",
        payload: serde_json::json!({}),
    })
    .await
    .unwrap();
    bus.prune(Some("main"), f64::MAX).await.unwrap();

    // Re-seed so oldest_id > 0.
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
