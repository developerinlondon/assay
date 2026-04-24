#![cfg(all(test, feature = "backend-sqlite"))]

use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use super::*;
use crate::events::{EngineEventBus, EventFilter, NewEvent, Subsystem};

async fn fresh_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::new()
        .filename(":memory:")
        .create_if_missing(true);
    // :memory: is per-connection; cap the pool at 1 so every query
    // targets the same instance.
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
