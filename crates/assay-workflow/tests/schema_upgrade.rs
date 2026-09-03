//! Opening a store on a database written by an earlier engine must succeed.
//!
//! `events.activity_id` arrived with its index in the baseline schema. On a
//! fresh database the two land together; on an existing one the table is
//! already there without the column, and the index statement runs before the
//! `ADD COLUMN`. The engine then exits at startup on every upgraded store.

mod common;

use assay_workflow::store::postgres::PostgresStore;
use assay_workflow::types::*;
use assay_workflow::{SqliteStore, WorkflowStore};
use common::harness::{TestPostgresDatabase, TestPostgresServer};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::str::FromStr;

fn workflow(id: &str) -> WorkflowRecord {
    WorkflowRecord {
        id: id.to_string(),
        namespace: "main".to_string(),
        run_id: "run-1".to_string(),
        workflow_type: "TestWorkflow".to_string(),
        task_queue: "upgrade-q".to_string(),
        status: "RUNNING".to_string(),
        input: None,
        result: None,
        error: None,
        parent_id: None,
        claimed_by: None,
        search_attributes: None,
        archived_at: None,
        archive_uri: None,
        created_at: 1.0,
        updated_at: 1.0,
        completed_at: None,
    }
}

/// A file-backed pool shaped like the engine's: `engine` and `workflow`
/// ATTACHed as files, so a second open sees what the first one wrote.
async fn file_pool(dir: &std::path::Path) -> SqlitePool {
    let engine = dir.join("engine.db").display().to_string();
    let workflow = dir.join("workflow.db").display().to_string();
    let opts =
        SqliteConnectOptions::from_str(&format!("sqlite://{}", dir.join("main.db").display()))
            .unwrap()
            .create_if_missing(true);
    SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(move |conn, _| {
            let engine = engine.clone();
            let workflow = workflow.clone();
            Box::pin(async move {
                use sqlx::Executor;
                conn.execute(format!("ATTACH DATABASE '{engine}' AS engine").as_str())
                    .await?;
                conn.execute(format!("ATTACH DATABASE '{workflow}' AS workflow").as_str())
                    .await?;
                Ok(())
            })
        })
        .connect_with(opts)
        .await
        .unwrap()
}

#[tokio::test]
async fn sqlite_store_opens_on_a_pre_activity_id_database() {
    let dir = tempfile::tempdir().unwrap();
    let pool = file_pool(dir.path()).await;

    let store = SqliteStore::from_attached_pool(pool.clone()).await.unwrap();
    store.create_workflow(&workflow("wf-old")).await.unwrap();
    drop(store);

    sqlx::raw_sql(
        "DROP INDEX workflow.idx_wf_events_activity;
         ALTER TABLE workflow.events DROP COLUMN activity_id;
         INSERT INTO workflow.events (workflow_id, seq, event_type, payload, timestamp)
         VALUES ('wf-old', 1, 'ActivityCompleted', '{\"activity_id\": 7}', 2.0);",
    )
    .execute(&pool)
    .await
    .unwrap();

    let reopened = SqliteStore::from_attached_pool(pool.clone())
        .await
        .expect("a store written by an earlier engine must open");
    let backfilled: Option<i64> =
        sqlx::query_scalar("SELECT activity_id FROM workflow.events WHERE workflow_id = 'wf-old'")
            .fetch_one(reopened.pool())
            .await
            .unwrap();
    assert_eq!(backfilled, Some(7));

    SqliteStore::from_attached_pool(pool)
        .await
        .expect("migration is idempotent");
}

#[tokio::test]
async fn postgres_store_opens_on_a_pre_activity_id_database() {
    let server = match TestPostgresServer::from_env_or_container().await {
        Ok(server) => server,
        Err(err) => {
            eprintln!("Skipping: Postgres unavailable: {err:#}");
            return;
        }
    };
    let database = TestPostgresDatabase::create(server).await.unwrap();
    let pool = database.pool().clone();

    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    store.create_workflow(&workflow("wf-old")).await.unwrap();
    drop(store);

    sqlx::raw_sql(
        "DROP INDEX workflow.idx_wf_events_activity;
         ALTER TABLE workflow.events DROP COLUMN activity_id;
         INSERT INTO workflow.events (workflow_id, seq, event_type, payload, timestamp)
         VALUES ('wf-old', 1, 'ActivityCompleted', '{\"activity_id\": 7}', 2.0);",
    )
    .execute(&pool)
    .await
    .unwrap();

    PostgresStore::from_pool(pool.clone())
        .await
        .expect("a store written by an earlier engine must open");
    let backfilled: Option<i64> =
        sqlx::query_scalar("SELECT activity_id FROM workflow.events WHERE workflow_id = 'wf-old'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(backfilled, Some(7));

    PostgresStore::from_pool(pool)
        .await
        .expect("migration is idempotent");
}
