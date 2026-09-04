//! Moving a populated SQLite store into Postgres.
//!
//! Drives a real engine over HTTP to write the store, runs the real
//! `migrate` subcommand, then reads the same rows back through a second
//! engine on Postgres. Needs `TEST_DATABASE_URL`; skipped without one.

mod common;

use common::{ADMIN_KEY, EngineProcess, ScratchDb, client, engine_binary};
use serde_json::json;
use std::path::Path;
use std::process::Output;

const SECRET: &str = "the-vault-must-survive-the-move";
const WORKFLOW_ID: &str = "migrated-run-1";

fn run_migrate(source: &Path, target: &str, extra: &[&str]) -> Output {
    let mut cmd = std::process::Command::new(engine_binary());
    cmd.arg("migrate")
        .arg("--from")
        .arg(source)
        .arg("--to")
        .arg(target);
    for arg in extra {
        cmd.arg(arg);
    }
    cmd.output().expect("run migrate")
}

fn sqlite_backend(dir: &Path) -> String {
    format!(
        "[backend]\ntype = \"sqlite\"\ndata_dir = \"{}\"",
        dir.display()
    )
}

fn pg_backend(url: &str) -> String {
    format!("[backend]\ntype = \"postgres\"\nurl = \"{url}\"")
}

/// A scratch Postgres and a populated SQLite store to migrate into it.
struct Fixture {
    db: ScratchDb,
    dir: tempfile::TempDir,
    data_dir: std::path::PathBuf,
}

/// `None` when no Postgres is configured, which every test reads as skip.
async fn fixture() -> Option<Fixture> {
    let db = ScratchDb::create().await?;
    let dir = tempfile::tempdir().expect("tempdir");
    let data_dir = dir.path().join("data");
    populate_sqlite_store(dir.path(), &data_dir).await;
    Some(Fixture { db, dir, data_dir })
}

/// Boot a SQLite engine, write a workflow run and a vault secret, and
/// shut it down. The store is left in `data_dir`.
async fn populate_sqlite_store(dir: &Path, data_dir: &Path) {
    let mut engine = EngineProcess::spawn(dir, "sqlite", &sqlite_backend(data_dir));
    let http = client();
    engine.wait_ready(&http).await.expect("sqlite engine ready");

    let r = http
        .post(engine.url("/api/v1/engine/workflow/workflows"))
        .header("Authorization", format!("Bearer {ADMIN_KEY}"))
        .json(&json!({"workflow_type": "IngestData", "workflow_id": WORKFLOW_ID}))
        .send()
        .await
        .expect("create workflow");
    assert_eq!(r.status(), 201, "create workflow: {}", engine.log());

    let r = http
        .put(engine.url("/api/v1/vault/kv/migrate/secret"))
        .header("Authorization", format!("Bearer {ADMIN_KEY}"))
        .json(&json!({"data": SECRET}))
        .send()
        .await
        .expect("write secret");
    assert_eq!(r.status(), 201, "write secret: {}", engine.log());

    engine.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn migrate_carries_workflows_and_the_vault_pg() {
    let Some(fx) = fixture().await else {
        eprintln!("skipped: TEST_DATABASE_URL not set");
        return;
    };

    let out = run_migrate(&fx.data_dir, &fx.db.url(), &[]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "migrate failed: {}\n{stdout}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("workflow.workflows") && stdout.contains("vault.kv"),
        "the report should count every table:\n{stdout}"
    );

    let mut engine = EngineProcess::spawn(fx.dir.path(), "pg", &pg_backend(&fx.db.url()));
    let http = client();
    engine.wait_ready(&http).await.expect("pg engine ready");
    let auth = format!("Bearer {ADMIN_KEY}");

    let r = http
        .get(engine.url(&format!("/api/v1/engine/workflow/workflows/{WORKFLOW_ID}")))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("get workflow");
    assert_eq!(r.status(), 200, "migrated run should be readable");
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["id"], WORKFLOW_ID);
    assert_eq!(body["workflow_type"], "IngestData");

    let r = http
        .get(engine.url(&format!(
            "/api/v1/engine/workflow/workflows/{WORKFLOW_ID}/events"
        )))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("get history");
    assert_eq!(r.status(), 200);
    let events: serde_json::Value = r.json().await.unwrap();
    assert!(
        !events.as_array().expect("history is an array").is_empty(),
        "the run's history should have come across too"
    );

    // The KEK lives in vault.kek_metadata, so reading this back on the
    // other backend is what proves the key travelled with the ciphertext.
    let r = http
        .get(engine.url("/api/v1/vault/kv/migrate/secret"))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("read secret");
    assert_eq!(r.status(), 200, "migrated secret should decrypt");
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["data"], SECRET);

    // Migrated ids are explicit, so a sequence still sitting at 1 would
    // collide here rather than at migration time.
    let r = http
        .post(engine.url("/api/v1/engine/workflow/workflows"))
        .header("Authorization", &auth)
        .json(&json!({"workflow_type": "IngestData", "workflow_id": "post-migration-run"}))
        .send()
        .await
        .expect("create workflow after migrating");
    assert_eq!(
        r.status(),
        201,
        "writing after a migration should not collide with migrated ids: {}",
        engine.log()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn migrate_refuses_a_target_that_holds_data_pg() {
    let Some(fx) = fixture().await else {
        eprintln!("skipped: TEST_DATABASE_URL not set");
        return;
    };

    assert!(
        run_migrate(&fx.data_dir, &fx.db.url(), &[]).status.success(),
        "first migration should succeed"
    );

    let out = run_migrate(&fx.data_dir, &fx.db.url(), &[]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!out.status.success(), "a second migration must be refused");
    assert!(
        stderr.contains("already holds engine data"),
        "refusal should say why:\n{stderr}"
    );
    assert!(
        stderr.contains("workflow.workflows"),
        "refusal should name the occupied tables:\n{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn migrate_dry_run_leaves_the_target_untouched_pg() {
    let Some(fx) = fixture().await else {
        eprintln!("skipped: TEST_DATABASE_URL not set");
        return;
    };

    let out = run_migrate(&fx.data_dir, &fx.db.url(), &["--dry-run"]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "dry run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("Dry run: nothing was written."),
        "dry run should say so:\n{stdout}"
    );
    assert!(
        stdout.contains("workflow.workflows"),
        "dry run should still count the source:\n{stdout}"
    );

    let pool = fx.db.pool().await;
    let schemas: Vec<(String,)> = sqlx::query_as(
        "SELECT schema_name FROM information_schema.schemata
         WHERE schema_name = ANY(ARRAY['engine','workflow','auth','vault'])",
    )
    .fetch_all(&pool)
    .await
    .expect("list schemas");
    assert!(
        schemas.is_empty(),
        "a dry run must create no schemas, found {schemas:?}"
    );
}
