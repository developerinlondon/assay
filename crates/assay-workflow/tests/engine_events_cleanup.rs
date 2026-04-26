//! Cleanup idempotency: a second prune after everything is gone
//! returns 0 rows affected. No cross-node coordination is needed
//! because DELETE WHERE ts < cutoff is a no-op when the row set is
//! empty.

#![cfg(feature = "backend-sqlite")]

use std::sync::Arc;

use assay_domain::events::{EngineEventBus, NewEvent, SqliteEngineEventBus, Subsystem};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

async fn fresh_bus() -> Arc<dyn EngineEventBus> {
    let opts = SqliteConnectOptions::new()
        .filename(":memory:")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    // v0.1.2: engine.events lives in an attached `engine` database.
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let alias = format!(
        "file:assay_evcleanup_{}_{}?mode=memory&cache=shared",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    );
    sqlx::query(&format!("ATTACH DATABASE '{alias}' AS engine"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS engine.events (
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
    Arc::new(SqliteEngineEventBus::new(pool).await.unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn cleanup_prunes_old_events_and_is_idempotent() {
    let bus = fresh_bus().await;
    for _ in 0..5 {
        bus.publish_committed(NewEvent {
            namespace: "main",
            subsystem: Subsystem::Workflow,
            kind: "x",
            payload: serde_json::json!({}),
        })
        .await
        .unwrap();
    }
    let n1 = bus.prune(f64::MAX).await.unwrap();
    assert_eq!(n1, 5);
    let n2 = bus.prune(f64::MAX).await.unwrap();
    assert_eq!(n2, 0, "prune must be idempotent");
}
