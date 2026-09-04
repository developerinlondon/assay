//! Concurrent first boot against one empty Postgres.
//!
//! `CREATE ... IF NOT EXISTS` is not atomic in Postgres — the existence
//! check precedes the catalog insert, so concurrent sessions can both
//! pass the check and one loses on `pg_type_typname_nsp_index`.
//! Needs a server in `TEST_DATABASE_URL`; skipped without one.

mod common;

use common::{EngineProcess, ScratchDb, client};

/// Well past the two engines that first showed the failure, so a change
/// that only narrows the window still fails here.
const INSTANCES: usize = 10;

/// Rounds of the whole experiment, each on its own empty database. The
/// race is timing-dependent, so one clean round is not evidence.
const ROUNDS: usize = 5;

/// Start `INSTANCES` engines together on one empty database and return
/// a description of each that failed to serve.
async fn one_round(round: usize) -> Vec<String> {
    let Some(db) = ScratchDb::create().await else {
        return Vec::new();
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = format!("[backend]\ntype = \"postgres\"\nurl = \"{}\"", db.url());

    // Nothing is awaited between spawns so every process reaches the
    // DDL inside the same few-millisecond window.
    let mut engines: Vec<EngineProcess> = (0..INSTANCES)
        .map(|i| EngineProcess::spawn(dir.path(), &format!("{round}-{i}"), &backend))
        .collect();

    let client = client();
    let mut failures = Vec::new();
    for (i, engine) in engines.iter_mut().enumerate() {
        if let Err(why) = engine.wait_ready(&client).await {
            failures.push(format!("--- round {round}, engine {i} ---\n{why}"));
        }
    }
    failures
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_first_boot_all_start_pg() {
    if ScratchDb::create().await.is_none() {
        eprintln!("skipped: TEST_DATABASE_URL not set");
        return;
    }
    let mut failures = Vec::new();
    for round in 0..ROUNDS {
        failures.extend(one_round(round).await);
    }
    assert!(
        failures.is_empty(),
        "{} engines failed to boot across {ROUNDS} rounds of {INSTANCES} simultaneous starts:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
