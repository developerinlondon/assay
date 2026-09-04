//! The engine must not touch tables it did not create.
//!
//! Its v0.13.1 relocation moves `public.workflows` and `public.namespaces`
//! into the `workflow` schema. Those are names an application owns too,
//! and `ALTER TABLE ... SET SCHEMA` survives a rollback, so a database
//! shared with a host application is the case that has to be safe.

mod common;

use common::{EngineProcess, ScratchDb, client};

/// An application's own tables, with columns nothing like the engine's.
const APP_SCHEMA: &str = "
CREATE TABLE public.workflows (
    id           BIGSERIAL PRIMARY KEY,
    tenant       TEXT NOT NULL,
    title        TEXT NOT NULL,
    approved_by  TEXT
);
INSERT INTO public.workflows (tenant, title, approved_by)
    SELECT 'acme', 'onboarding ' || g, 'someone'
    FROM generate_series(1, 30) g;

CREATE TABLE public.namespaces (
    slug      TEXT PRIMARY KEY,
    owner_id  BIGINT NOT NULL
);
INSERT INTO public.namespaces (slug, owner_id) VALUES ('acme', 1), ('globex', 2);

CREATE TABLE public.api_keys (
    id     BIGSERIAL PRIMARY KEY,
    label  TEXT NOT NULL
);
INSERT INTO public.api_keys (label) VALUES ('billing'), ('reporting');
";

async fn table_exists(pool: &sqlx::PgPool, schema: &str, table: &str) -> bool {
    sqlx::query_scalar::<_, Option<String>>(&format!(
        "SELECT to_regclass('{schema}.{table}')::text"
    ))
    .fetch_one(pool)
    .await
    .expect("to_regclass")
    .is_some()
}

/// Boot the engine on a database whose `public` schema belongs to an
/// application. Its tables, rows and columns must all still be there.
#[tokio::test(flavor = "multi_thread")]
async fn engine_leaves_an_applications_public_tables_alone_pg() {
    let Some(db) = ScratchDb::create().await else {
        eprintln!("skipped: TEST_DATABASE_URL not set");
        return;
    };
    let pool = db.pool().await;
    sqlx::raw_sql(APP_SCHEMA)
        .execute(&pool)
        .await
        .expect("create the application's tables");

    let dir = tempfile::tempdir().expect("tempdir");
    let backend = format!("[backend]\ntype = \"postgres\"\nurl = \"{}\"", db.url());
    let mut engine = EngineProcess::spawn(dir.path(), "host-app", &backend);
    engine
        .wait_ready(&client())
        .await
        .expect("engine should boot on a shared database");

    for table in ["workflows", "namespaces", "api_keys"] {
        assert!(
            table_exists(&pool, "public", table).await,
            "public.{table} was taken or dropped by the engine"
        );
    }

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM public.workflows")
        .fetch_one(&pool)
        .await
        .expect("count the application's rows");
    assert_eq!(rows, 30, "the application's rows must survive");

    let approved: Option<String> =
        sqlx::query_scalar("SELECT approved_by FROM public.workflows ORDER BY id LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("the application's own columns must survive");
    assert_eq!(approved.as_deref(), Some("someone"));

    let slugs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM public.namespaces")
        .fetch_one(&pool)
        .await
        .expect("count the application's namespaces");
    assert_eq!(slugs, 2);

    // The engine's own tables land beside them, not on top of them.
    assert!(
        table_exists(&pool, "workflow", "workflows").await,
        "the engine should still create its own workflow.workflows"
    );
    let engine_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workflow.workflows")
        .fetch_one(&pool)
        .await
        .expect("count engine rows");
    assert_eq!(
        engine_rows, 0,
        "the engine's table should be empty, not the application's 30 rows moved into it"
    );
}

/// A real v0.13.1 store still relocates. The prefixed tables are the
/// provenance marker, so their presence is what unlocks the move.
#[tokio::test(flavor = "multi_thread")]
async fn a_genuine_legacy_store_still_relocates_pg() {
    let Some(db) = ScratchDb::create().await else {
        eprintln!("skipped: TEST_DATABASE_URL not set");
        return;
    };
    let pool = db.pool().await;
    sqlx::raw_sql(
        "
        CREATE TABLE public.workflows (
            id TEXT PRIMARY KEY, namespace TEXT NOT NULL DEFAULT 'main',
            run_id TEXT NOT NULL, workflow_type TEXT NOT NULL,
            task_queue TEXT NOT NULL DEFAULT 'main',
            status TEXT NOT NULL DEFAULT 'PENDING',
            created_at DOUBLE PRECISION NOT NULL DEFAULT 0,
            updated_at DOUBLE PRECISION NOT NULL DEFAULT 0
        );
        INSERT INTO public.workflows (id, run_id, workflow_type)
            VALUES ('legacy-1', 'run-legacy-1', 'IngestData');
        CREATE TABLE public.workflow_events (
            id BIGSERIAL PRIMARY KEY, workflow_id TEXT NOT NULL,
            seq INTEGER NOT NULL, event_type TEXT NOT NULL,
            payload TEXT, timestamp DOUBLE PRECISION NOT NULL DEFAULT 0
        );
        ",
    )
    .execute(&pool)
    .await
    .expect("create a v0.13.1-shaped store");

    let dir = tempfile::tempdir().expect("tempdir");
    let backend = format!("[backend]\ntype = \"postgres\"\nurl = \"{}\"", db.url());
    let mut engine = EngineProcess::spawn(dir.path(), "legacy", &backend);
    engine
        .wait_ready(&client())
        .await
        .expect("engine should boot on a legacy store");

    assert!(
        !table_exists(&pool, "public", "workflows").await,
        "a genuine legacy public.workflows should have moved"
    );
    let moved: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workflow.workflows")
        .fetch_one(&pool)
        .await
        .expect("count relocated rows");
    assert_eq!(moved, 1, "the legacy row should have come across");
}
