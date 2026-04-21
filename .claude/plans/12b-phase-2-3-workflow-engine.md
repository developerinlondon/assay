# 12b — Phases 2 + 3 — Workflow trait cleanup and SurrealDB backend

> Sub-plan of [12-v0.13.0-execution.md](./12-v0.13.0-execution.md). Prerequisites: phases 0 + 1 from
> [12a](./12a-phase-0-1-workspace-and-state.md).

**Phase 2 goal:** ensure `WorkflowStore` contract is complete and parametrised testing is ready.
Most of Phase 2 was absorbed into Phase 1 (trait moved to `assay-core`, push-stream signatures
added, backends feature-gated). What remains is a parametrised test harness.

**Phase 3 goal:** `SurrealDbStore` implements every `WorkflowStore` method. Every existing
integration test passes against SurrealDB via the parametrised harness. Scheduler wakes via
SurrealDB `LIVE SELECT`.

---

## Phase 2 — Workflow trait cleanup + parametrised test harness

### Task 2.1: Parametrised test harness across PG / SQLite / SurrealDB

**Files:**

- Create: `crates/assay-workflow/tests/common/mod.rs`
- Create: `crates/assay-workflow/tests/common/harness.rs`
- Modify: `crates/assay-workflow/Cargo.toml` (dev-deps)

Each existing test gets annotated with `#[rstest]` cases for all three backends. SurrealDB cases
will fail until Phase 3 lands — that's the driving test suite for Phase 3.

- [ ] **Step 1: Add dev-dependencies**

```toml
# crates/assay-workflow/Cargo.toml [dev-dependencies]
rstest = "0.26"
testcontainers = "0.27"
testcontainers-modules = { version = "0.15", features = ["postgres", "surrealdb"] }
tokio = { version = "1", features = ["full"] }
tempfile = "3"
```

- [ ] **Step 2: Write the harness**

```rust
// crates/assay-workflow/tests/common/harness.rs
//! Parametrised backend harness. Each test fn is decorated with rstest
//! cases for PG, SQLite, and SurrealDB; the test body is generic over
//! `&dyn WorkflowStore`.

use std::sync::Arc;
use assay_core::WorkflowStore;

#[cfg(feature = "backend-postgres")]
use assay_workflow::PostgresStore;
#[cfg(feature = "backend-sqlite")]
use assay_workflow::SqliteStore;
#[cfg(feature = "backend-surrealdb")]
use assay_workflow::SurrealDbStore;

pub enum Harness {
    #[cfg(feature = "backend-postgres")]
    Postgres {
        _container: testcontainers::ContainerAsync<
            testcontainers_modules::postgres::Postgres>,
        store: Arc<PostgresStore>,
    },
    #[cfg(feature = "backend-sqlite")]
    Sqlite {
        _tempdir: tempfile::TempDir,
        store: Arc<SqliteStore>,
    },
    #[cfg(feature = "backend-surrealdb")]
    Surreal {
        _container: testcontainers::ContainerAsync<
            testcontainers_modules::surrealdb::SurrealDB>,
        store: Arc<SurrealDbStore>,
    },
}

impl Harness {
    #[cfg(feature = "backend-postgres")]
    pub async fn postgres() -> anyhow::Result<Self> {
        use testcontainers::runners::AsyncRunner;
        use testcontainers_modules::postgres::Postgres as PgImage;

        let container = PgImage::default().start().await?;
        let host = container.get_host().await?;
        let port = container.get_host_port_ipv4(5432).await?;
        let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

        let store = PostgresStore::connect(&url).await?;
        store.create_namespace("main").await?;
        Ok(Self::Postgres { _container: container, store: Arc::new(store) })
    }

    #[cfg(feature = "backend-sqlite")]
    pub async fn sqlite() -> anyhow::Result<Self> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("assay.db");
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let store = SqliteStore::connect(&url).await?;
        store.create_namespace("main").await?;
        Ok(Self::Sqlite { _tempdir: dir, store: Arc::new(store) })
    }

    #[cfg(feature = "backend-surrealdb")]
    pub async fn surreal() -> anyhow::Result<Self> {
        use testcontainers::runners::AsyncRunner;
        use testcontainers_modules::surrealdb::SurrealDB;

        let container = SurrealDB::default()
            .with_user("root")
            .with_password("root")
            .start()
            .await?;
        let host = container.get_host().await?;
        let port = container.get_host_port_ipv4(8000).await?;
        let url = format!("ws://{host}:{port}");

        let store = SurrealDbStore::connect_full(
            &url, "assay", "workflow", Some("root"), Some("root"),
        ).await?;
        store.create_namespace("main").await?;
        Ok(Self::Surreal { _container: container, store: Arc::new(store) })
    }

    pub fn store(&self) -> Arc<dyn WorkflowStore> {
        match self {
            #[cfg(feature = "backend-postgres")]
            Self::Postgres { store, .. } => store.clone(),
            #[cfg(feature = "backend-sqlite")]
            Self::Sqlite { store, .. } => store.clone(),
            #[cfg(feature = "backend-surrealdb")]
            Self::Surreal { store, .. } => store.clone(),
        }
    }
}

pub enum Backend { Postgres, Sqlite, Surreal }

impl Backend {
    pub async fn setup(self) -> anyhow::Result<Harness> {
        match self {
            #[cfg(feature = "backend-postgres")]  Self::Postgres => Harness::postgres().await,
            #[cfg(feature = "backend-sqlite")]    Self::Sqlite   => Harness::sqlite().await,
            #[cfg(feature = "backend-surrealdb")] Self::Surreal  => Harness::surreal().await,
            #[allow(unreachable_patterns)] _ => anyhow::bail!("backend feature disabled"),
        }
    }
}
```

```rust
// crates/assay-workflow/tests/common/mod.rs
pub mod harness;
pub use harness::*;
```

- [ ] **Step 3: Smoke test across backends**

```rust
// crates/assay-workflow/tests/smoke_backends.rs
mod common;
use common::{Backend, Harness};
use rstest::rstest;

#[rstest]
#[case::pg(Backend::Postgres)]
#[case::sqlite(Backend::Sqlite)]
#[case::surreal(Backend::Surreal)]
#[tokio::test(flavor = "multi_thread")]
async fn namespace_roundtrip(#[case] backend: Backend) {
    let h = backend.setup().await.unwrap();
    let store = h.store();
    let list = store.list_namespaces().await.unwrap();
    assert!(list.iter().any(|n| n.name == "main"));
}
```

- [ ] **Step 4: Run — confirm PG + SQLite pass, Surreal fails**

```bash
cargo test -p assay-workflow --test smoke_backends \
    --features "backend-postgres backend-sqlite backend-surrealdb" \
    -- --nocapture
```

Expected: two cases green (pg, sqlite), one case red (surreal with
`SurrealDbStore::connect_full — not yet implemented`).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "test(workflow): parametrised backend harness (pg, sqlite, surreal)

Surreal case fails with 'not implemented' — drives Phase 3's
SurrealDbStore implementation."
```

---

### Phase 2 exit criteria

- Harness compiles and one smoke test runs.
- PG and SQLite cases green; Surreal red with a clear failure mode.

---

## Phase 3 — SurrealDB workflow backend

**Approach:** 15 sub-tasks, each mirroring one group of `WorkflowStore` methods from the PG impl
(`crates/assay-workflow/src/store/postgres.rs`). Each sub-task is a self-contained commit; the
parametrised test harness drives TDD for every one.

**Per-sub-task shape:**

1. Add the Surreal case to the relevant existing test (if not yet included in harness smoke
   coverage).
2. Run — expect failure (`unimplemented!` or `todo!()`).
3. Implement the method(s) on `SurrealDbStore`, mirroring PG semantics. Reference plan 10 §
   "SurrealDB backend specifics" for the surql pattern.
4. Run the test — expect pass across all three backends.
5. Commit with message `feat(workflow/surreal): <operation>`.

**Full SurrealDB reference material:** plan 10 lines 232–335 cover schema, transactions, dispatch,
wake-up hybrid model, and migration tool for SurrealDB. Each sub-task below cites the specific
section.

### Task 3.1: SurrealDbStore connection + migrations runner

**Files:**

- Modify: `crates/assay-workflow/src/store/surrealdb.rs` (replace the stub)
- Create: `crates/assay-workflow/migrations/surrealdb/00_init.surql`
- Create: `crates/assay-workflow/src/store/surrealdb/migrations.rs`

- [ ] **Step 1: Replace the connect stub**

```rust
// crates/assay-workflow/src/store/surrealdb.rs
use std::sync::Arc;
use surrealdb::engine::remote::ws::{Client, Ws, Wss};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;

pub struct SurrealDbStore {
    pub(crate) db: Arc<Surreal<Client>>,
}

impl SurrealDbStore {
    pub async fn connect_full(
        url: &str,
        namespace: &str,
        database: &str,
        username: Option<&str>,
        password: Option<&str>,
    ) -> anyhow::Result<Self> {
        let db = if url.starts_with("wss://") {
            Surreal::new::<Wss>(url.trim_start_matches("wss://")).await?
        } else {
            Surreal::new::<Ws>(url.trim_start_matches("ws://")).await?
        };

        if let (Some(u), Some(p)) = (username, password) {
            db.signin(Root { username: u, password: p }).await?;
        }

        db.use_ns(namespace).use_db(database).await?;

        let this = Self { db: Arc::new(db) };
        this.run_migrations().await?;
        Ok(this)
    }

    /// Convenience for consumers who don't need auth.
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        Self::connect_full(url, "assay", "workflow", None, None).await
    }
}

mod migrations;
```

- [ ] **Step 2: Migrations runner**

```rust
// crates/assay-workflow/src/store/surrealdb/migrations.rs
use super::SurrealDbStore;

const MIGRATIONS: &[(&str, &str)] = &[
    ("00_init", include_str!("../../../migrations/surrealdb/00_init.surql")),
];

impl SurrealDbStore {
    pub(crate) async fn run_migrations(&self) -> anyhow::Result<()> {
        self.db
            .query("
                DEFINE TABLE IF NOT EXISTS _assay_migrations SCHEMAFULL;
                DEFINE FIELD IF NOT EXISTS name ON _assay_migrations TYPE string;
                DEFINE FIELD IF NOT EXISTS applied_at ON _assay_migrations TYPE datetime DEFAULT time::now();
                DEFINE INDEX IF NOT EXISTS name_unique ON _assay_migrations COLUMNS name UNIQUE;
            ")
            .await?;

        for (name, sql) in MIGRATIONS {
            let applied: Option<String> = self.db
                .query("SELECT name FROM _assay_migrations WHERE name = $name LIMIT 1")
                .bind(("name", name.to_string()))
                .await?
                .take::<Option<String>>(0)?;
            if applied.is_some() {
                continue;
            }
            self.db.query(*sql).await?;
            self.db
                .query("CREATE _assay_migrations SET name = $name")
                .bind(("name", name.to_string()))
                .await?;
        }
        Ok(())
    }
}
```

- [ ] **Step 3: Write initial schema migration**

```surql
-- crates/assay-workflow/migrations/surrealdb/00_init.surql

-- Namespaces
DEFINE TABLE IF NOT EXISTS namespace SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name ON namespace TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON namespace TYPE float;
DEFINE INDEX IF NOT EXISTS namespace_name ON namespace COLUMNS name UNIQUE;

-- Workflows
DEFINE TABLE IF NOT EXISTS workflow SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS id ON workflow TYPE string;
DEFINE FIELD IF NOT EXISTS namespace ON workflow TYPE string;
DEFINE FIELD IF NOT EXISTS status ON workflow TYPE string;
DEFINE FIELD IF NOT EXISTS workflow_type ON workflow TYPE string;
DEFINE FIELD IF NOT EXISTS search_attributes ON workflow TYPE option<object>;
DEFINE FIELD IF NOT EXISTS next_dispatch_at ON workflow TYPE option<float>;
DEFINE FIELD IF NOT EXISTS needs_dispatch ON workflow TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS dispatch_claimed_by ON workflow TYPE option<string>;
DEFINE FIELD IF NOT EXISTS dispatch_last_heartbeat ON workflow TYPE option<float>;
DEFINE FIELD IF NOT EXISTS version ON workflow TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS completed_at ON workflow TYPE option<float>;
DEFINE FIELD IF NOT EXISTS archived_at ON workflow TYPE option<float>;
DEFINE FIELD IF NOT EXISTS archive_uri ON workflow TYPE option<string>;
DEFINE FIELD IF NOT EXISTS parent_id ON workflow TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at ON workflow TYPE float;
DEFINE INDEX IF NOT EXISTS workflow_dispatch ON workflow COLUMNS namespace, status, next_dispatch_at;
DEFINE INDEX IF NOT EXISTS workflow_parent ON workflow COLUMNS parent_id;

-- Events
DEFINE TABLE IF NOT EXISTS event SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS workflow_id ON event TYPE string;
DEFINE FIELD IF NOT EXISTS seq ON event TYPE int;
DEFINE FIELD IF NOT EXISTS event_type ON event TYPE string;
DEFINE FIELD IF NOT EXISTS payload ON event TYPE option<object>;
DEFINE FIELD IF NOT EXISTS created_at ON event TYPE float;
DEFINE INDEX IF NOT EXISTS event_workflow ON event COLUMNS workflow_id, seq;

-- Activities
DEFINE TABLE IF NOT EXISTS activity SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS workflow_id ON activity TYPE string;
DEFINE FIELD IF NOT EXISTS seq ON activity TYPE int;
DEFINE FIELD IF NOT EXISTS task_queue ON activity TYPE string;
DEFINE FIELD IF NOT EXISTS status ON activity TYPE string;
DEFINE FIELD IF NOT EXISTS attempt ON activity TYPE int DEFAULT 1;
DEFINE FIELD IF NOT EXISTS scheduled_at ON activity TYPE float;
DEFINE FIELD IF NOT EXISTS claimed_by ON activity TYPE option<string>;
DEFINE FIELD IF NOT EXISTS started_at ON activity TYPE option<float>;
DEFINE FIELD IF NOT EXISTS heartbeat_at ON activity TYPE option<float>;
DEFINE FIELD IF NOT EXISTS heartbeat_details ON activity TYPE option<string>;
DEFINE FIELD IF NOT EXISTS result ON activity TYPE option<string>;
DEFINE FIELD IF NOT EXISTS error ON activity TYPE option<string>;
DEFINE INDEX IF NOT EXISTS activity_claim ON activity COLUMNS task_queue, status, scheduled_at;
DEFINE INDEX IF NOT EXISTS activity_workflow_seq ON activity COLUMNS workflow_id, seq UNIQUE;

-- Timers
DEFINE TABLE IF NOT EXISTS timer SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS workflow_id ON timer TYPE string;
DEFINE FIELD IF NOT EXISTS seq ON timer TYPE int;
DEFINE FIELD IF NOT EXISTS fire_at ON timer TYPE float;
DEFINE FIELD IF NOT EXISTS fired ON timer TYPE bool DEFAULT false;
DEFINE INDEX IF NOT EXISTS timer_fire_at ON timer COLUMNS fire_at, fired;
DEFINE INDEX IF NOT EXISTS timer_workflow_seq ON timer COLUMNS workflow_id, seq UNIQUE;

-- Signals
DEFINE TABLE IF NOT EXISTS signal SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS workflow_id ON signal TYPE string;
DEFINE FIELD IF NOT EXISTS name ON signal TYPE string;
DEFINE FIELD IF NOT EXISTS payload ON signal TYPE option<object>;
DEFINE FIELD IF NOT EXISTS consumed ON signal TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS created_at ON signal TYPE float;
DEFINE INDEX IF NOT EXISTS signal_consume ON signal COLUMNS workflow_id, name, consumed;

-- Schedules
DEFINE TABLE IF NOT EXISTS schedule SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS namespace ON schedule TYPE string;
DEFINE FIELD IF NOT EXISTS name ON schedule TYPE string;
DEFINE FIELD IF NOT EXISTS cron_expr ON schedule TYPE string;
DEFINE FIELD IF NOT EXISTS timezone ON schedule TYPE string;
DEFINE FIELD IF NOT EXISTS paused ON schedule TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS last_run_at ON schedule TYPE option<float>;
DEFINE FIELD IF NOT EXISTS next_run_at ON schedule TYPE option<float>;
DEFINE INDEX IF NOT EXISTS schedule_ns_name ON schedule COLUMNS namespace, name UNIQUE;
DEFINE INDEX IF NOT EXISTS schedule_next_run ON schedule COLUMNS next_run_at;

-- Snapshots
DEFINE TABLE IF NOT EXISTS snapshot SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS workflow_id ON snapshot TYPE string;
DEFINE FIELD IF NOT EXISTS event_seq ON snapshot TYPE int;
DEFINE FIELD IF NOT EXISTS state_json ON snapshot TYPE string;
DEFINE INDEX IF NOT EXISTS snapshot_workflow_seq ON snapshot COLUMNS workflow_id, event_seq;

-- Workers
DEFINE TABLE IF NOT EXISTS worker SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS id ON worker TYPE string;
DEFINE FIELD IF NOT EXISTS namespace ON worker TYPE string;
DEFINE FIELD IF NOT EXISTS task_queues ON worker TYPE array<string>;
DEFINE FIELD IF NOT EXISTS heartbeat_at ON worker TYPE float;
DEFINE INDEX IF NOT EXISTS worker_ns ON worker COLUMNS namespace;

-- API keys
DEFINE TABLE IF NOT EXISTS api_key SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS key_hash ON api_key TYPE string;
DEFINE FIELD IF NOT EXISTS prefix ON api_key TYPE string;
DEFINE FIELD IF NOT EXISTS label ON api_key TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at ON api_key TYPE float;
DEFINE INDEX IF NOT EXISTS api_key_prefix ON api_key COLUMNS prefix UNIQUE;
DEFINE INDEX IF NOT EXISTS api_key_hash ON api_key COLUMNS key_hash UNIQUE;

-- Scheduler lock (for leader election)
DEFINE TABLE IF NOT EXISTS scheduler_lock SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS holder ON scheduler_lock TYPE string;
DEFINE FIELD IF NOT EXISTS expires_at ON scheduler_lock TYPE float;
```

- [ ] **Step 4: Run — harness connects**

```bash
cargo test -p assay-workflow --test smoke_backends --features backend-surrealdb -- --nocapture
```

Surreal case still fails (the `create_namespace` trait method is `todo!()`), but the harness now
reaches the call — connection + migrations succeed.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(workflow/surreal): connect + migrations runner

Embedded .surql migrations applied via _assay_migrations tracker.
Schema covers namespace, workflow, event, activity, timer, signal,
schedule, snapshot, worker, api_key, scheduler_lock. WorkflowStore
trait methods are todo!() until subsequent tasks."
```

---

### Task 3.2: Namespaces (create, list, delete, stats)

**Mirror:** `crates/assay-workflow/src/store/postgres.rs` — `create_namespace`, `list_namespaces`,
`delete_namespace`, `get_namespace_stats`.

- [ ] **Step 1: Write parametrised namespace CRUD tests**

Extend `crates/assay-workflow/tests/smoke_backends.rs` with a `list_namespaces_after_create` and
`delete_namespace_removes_it` test.

- [ ] **Step 2: Run — Surreal cases fail**

- [ ] **Step 3: Implement on `SurrealDbStore`**

```rust
// crates/assay-workflow/src/store/surrealdb.rs
// (impl block begins — will grow through tasks 3.2..3.16)

use async_trait::async_trait;  // only if needed; trait uses `impl Future` so not required
use assay_core::{NamespaceRecord, NamespaceStats, WorkflowStore};

impl WorkflowStore for SurrealDbStore {
    fn create_namespace(&self, name: &str) -> impl Future<Output = anyhow::Result<()>> + Send + '_ {
        let name = name.to_string();
        async move {
            self.db
                .query("CREATE namespace:⟨$name⟩ SET name = $name, created_at = time::now()")
                .bind(("name", name))
                .await?;
            Ok(())
        }
    }

    fn list_namespaces(&self) -> impl Future<Output = anyhow::Result<Vec<NamespaceRecord>>> + Send + '_ {
        async move {
            let rows: Vec<NamespaceRecord> = self.db
                .query("SELECT name, created_at FROM namespace ORDER BY created_at ASC")
                .await?
                .take(0)?;
            Ok(rows)
        }
    }

    fn delete_namespace(&self, name: &str) -> impl Future<Output = anyhow::Result<bool>> + Send + '_ {
        let name = name.to_string();
        async move {
            let removed: Option<NamespaceRecord> = self.db
                .query("DELETE namespace WHERE name = $name RETURN BEFORE")
                .bind(("name", name))
                .await?
                .take::<Option<NamespaceRecord>>(0)?;
            Ok(removed.is_some())
        }
    }

    fn get_namespace_stats(&self, namespace: &str) -> impl Future<Output = anyhow::Result<NamespaceStats>> + Send + '_ {
        let ns = namespace.to_string();
        async move {
            // Aggregate count per status
            let stats: Option<NamespaceStats> = self.db
                .query("
                    LET $total = (SELECT count() FROM workflow WHERE namespace = $ns GROUP ALL);
                    LET $running = (SELECT count() FROM workflow WHERE namespace = $ns AND status = 'running' GROUP ALL);
                    LET $pending = (SELECT count() FROM workflow WHERE namespace = $ns AND status = 'pending' GROUP ALL);
                    LET $completed = (SELECT count() FROM workflow WHERE namespace = $ns AND status = 'completed' GROUP ALL);
                    LET $failed = (SELECT count() FROM workflow WHERE namespace = $ns AND status = 'failed' GROUP ALL);
                    LET $schedules = (SELECT count() FROM schedule WHERE namespace = $ns GROUP ALL);
                    LET $workers = (SELECT count() FROM worker WHERE namespace = $ns GROUP ALL);
                    RETURN { namespace: $ns, total_workflows: $total[0].count ?? 0, running: $running[0].count ?? 0, pending: $pending[0].count ?? 0, completed: $completed[0].count ?? 0, failed: $failed[0].count ?? 0, schedules: $schedules[0].count ?? 0, workers: $workers[0].count ?? 0 };
                ")
                .bind(("ns", ns))
                .await?
                .take(0)?;
            stats.ok_or_else(|| anyhow::anyhow!("stats query returned nothing"))
        }
    }

    // ... remaining methods are todo!() until their task lands
    fn create_workflow(&self, _w: &assay_core::WorkflowRecord) -> impl Future<Output = anyhow::Result<()>> + Send + '_ {
        async move { todo!("Task 3.3") }
    }
    // ... (stub out every remaining trait method with `todo!("Task N")` to name the gap)
}
```

(Every trait method not yet implemented is stubbed with `todo!("Task N")` — don't leave them
unimplemented at compile time, the trait demands them. Subsequent tasks flesh them out.)

- [ ] **Step 4: Run — namespace tests green across all three backends**

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(workflow/surreal): namespaces (create/list/delete/stats)"
```

---

### Tasks 3.3 – 3.15: Remaining `WorkflowStore` methods on SurrealDB

Each task follows the same shape as Task 3.2: extend tests, run Surreal case fails, implement
methods mirroring PG, commit.

**Key surql patterns to reuse:**

- Inserting with optimistic concurrency:
  `UPDATE workflow:⟨$id⟩ SET status = $new_status, version = version + 1 WHERE version = $expected_version`
  — if the result is empty, retry.
- Atomic claim:
  `BEGIN TRANSACTION; LET $claimed = (SELECT * FROM workflow WHERE ...conditions... LIMIT 1); IF $claimed != NONE THEN UPDATE $claimed SET dispatch_claimed_by = $worker, dispatch_last_heartbeat = time::now() END; COMMIT;`
- Event append:
  `CREATE event CONTENT { workflow_id: $wf, seq: $seq, event_type: $type, payload: $payload, created_at: time::now() }`.
- Range delete on archival:
  `DELETE event WHERE workflow_id = $wf; DELETE activity WHERE workflow_id = $wf; DELETE timer WHERE workflow_id = $wf; DELETE signal WHERE workflow_id = $wf; DELETE snapshot WHERE workflow_id = $wf; UPDATE workflow:⟨$wf⟩ SET archived_at = $now, archive_uri = $uri`.

| #    | Method group                                             | PG reference (line-range approx)                                                                                                                                            | Surreal key pattern                                                                                                                                      |
| ---- | -------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 3.3  | Workflows CRUD + status + claim + dispatch hints         | `postgres.rs` create/get/list/update_workflow_status, claim_workflow, mark_workflow_dispatchable, claim_workflow_task, release_workflow_task, release_stale_dispatch_leases | `UPDATE` + `version` field optimistic concurrency                                                                                                        |
| 3.4  | Events (append/list/count)                               | `postgres.rs` `append_event`, `list_events`, `get_event_count`                                                                                                              | Insert with computed `seq = (SELECT count() FROM event WHERE workflow_id=$w)+1` inside a transaction, or hold seq at workflow level                      |
| 3.5  | Search attributes (upsert, filter)                       | `postgres.rs` `upsert_search_attributes`, filter in `list_workflows`                                                                                                        | `UPDATE workflow:⟨$id⟩ SET search_attributes = object::extend(search_attributes ?? {}, $patch)`                                                          |
| 3.6  | Activities (create/get/claim/complete/heartbeat/requeue) | `postgres.rs` `{create,get,claim,complete,heartbeat,requeue}_activity`, `get_timed_out_activities`, `cancel_pending_activities`                                             | `activity_claim` index: `UPDATE activity WHERE task_queue=$q AND status='pending' AND scheduled_at <= time::now() ORDER BY scheduled_at LIMIT 1 SET ...` |
| 3.7  | Timers (create, fire, cancel)                            | `postgres.rs` `create_timer`, `fire_due_timers`, `cancel_pending_timers`, `get_timer_by_workflow_seq`                                                                       | `timer_fire_at` index; `UPDATE timer WHERE fire_at <= time::now() AND fired=false SET fired=true RETURN BEFORE`                                          |
| 3.8  | Signals (send/consume)                                   | `postgres.rs` `send_signal`, `consume_signals`                                                                                                                              | `UPDATE signal WHERE workflow_id=$w AND name=$n AND consumed=false SET consumed=true RETURN BEFORE`                                                      |
| 3.9  | Schedules (CRUD, paused, next-run)                       | `postgres.rs` all `*_schedule*` methods                                                                                                                                     | `UPDATE schedule:⟨$ns:$name⟩` for mutations                                                                                                              |
| 3.10 | Snapshots                                                | `postgres.rs` `create_snapshot`, `get_latest_snapshot`                                                                                                                      | `SELECT * FROM snapshot WHERE workflow_id=$w ORDER BY event_seq DESC LIMIT 1`                                                                            |
| 3.11 | Archival + purge                                         | `postgres.rs` `list_archivable_workflows`, `mark_archived_and_purge`                                                                                                        | Delete-by-workflow pattern above + `archived_at`/`archive_uri` on workflow                                                                               |
| 3.12 | Workers (register/heartbeat/list/remove_dead)            | `postgres.rs` `register_worker`, `heartbeat_worker`, `list_workers`, `remove_dead_workers`                                                                                  | `UPSERT worker:⟨$id⟩ SET ...`, periodic sweep for cutoff                                                                                                 |
| 3.13 | API keys (CRUD, idempotent, empty check)                 | `postgres.rs` `*_api_key*` methods                                                                                                                                          | Direct table ops; use index on `key_hash` for `validate_api_key`                                                                                         |
| 3.14 | Queue stats + child workflows                            | `postgres.rs` `get_queue_stats`, `list_child_workflows`                                                                                                                     | Aggregate queries on `activity` + `worker`, parent_id index                                                                                              |
| 3.15 | Leader election                                          | `postgres.rs` `try_acquire_scheduler_lock`                                                                                                                                  | Check expired lock → UPSERT with expiry; SELECT current holder                                                                                           |

**For every task in 3.3 – 3.15:**

- Each implementation goes in a new `impl` block section within
  `crates/assay-workflow/src/store/surrealdb.rs` (or a sub-module
  `crates/assay-workflow/src/store/surrealdb/<area>.rs` if the file grows past 800 lines).
- Tests extend the parametrised harness. Reuse existing PG-specific integration tests when possible
  — they move to use the harness.
- Commit per task: `feat(workflow/surreal): <method-group>`.

---

### Task 3.16: Push streams via `LIVE SELECT`

**Files:**

- Modify: `crates/assay-workflow/src/store/surrealdb.rs` (replace the empty-stream stubs)

- [ ] **Step 1: Write a push-delivery integration test**

```rust
// crates/assay-workflow/tests/surreal_push.rs
mod common;
use common::{Backend, Harness};
use futures_util::StreamExt;

#[tokio::test(flavor = "multi_thread")]
async fn surreal_runnable_push_fires_on_insert() {
    let h = Backend::Surreal.setup().await.unwrap();
    let store = h.store();

    let mut stream = Box::pin(store.subscribe_runnable("main"));

    // Spawn a task that inserts a runnable workflow after a short delay,
    // so the live select subscription is in place first.
    let store_bg = store.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let wf = assay_core::WorkflowRecord {
            id: "wf-push-1".into(),
            namespace: "main".into(),
            status: assay_core::WorkflowStatus::Running,  // treated as runnable
            // ... other fields default
            ..Default::default()
        };
        store_bg.create_workflow(&wf).await.unwrap();
    });

    let id = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream.next(),
    ).await.expect("timed out").expect("stream ended early");

    assert_eq!(id, "wf-push-1");
}
```

- [ ] **Step 2: Implement `subscribe_runnable` via LIVE**

```rust
// crates/assay-workflow/src/store/surrealdb.rs

fn subscribe_runnable(&self, namespace: &str)
    -> impl futures_core::Stream<Item = String> + Send + '_
{
    let db = self.db.clone();
    let ns = namespace.to_string();
    async_stream::stream! {
        let result = db
            .query("LIVE SELECT id FROM workflow WHERE namespace = $ns AND status IN ['running', 'pending']")
            .bind(("ns", ns))
            .await;

        let mut stream = match result {
            Ok(mut r) => match r.stream::<surrealdb::Notification<serde_json::Value>>(0) {
                Ok(s) => s,
                Err(e) => { tracing::error!(?e, "LIVE stream init failed"); return; }
            },
            Err(e) => { tracing::error!(?e, "LIVE query failed"); return; }
        };

        while let Some(notif) = stream.next().await {
            match notif {
                Ok(n) => {
                    if matches!(n.action, surrealdb::Action::Create | surrealdb::Action::Update) {
                        if let Some(id) = n.data.get("id").and_then(|v| v.as_str()) {
                            yield id.to_string();
                        }
                    }
                }
                Err(e) => tracing::warn!(?e, "LIVE notification error"),
            }
        }
    }
}
```

Add `async-stream = "0.3"` and `futures-util = "0.3"` to `crates/assay-workflow/Cargo.toml` if not
already present.

- [ ] **Step 3: Same pattern for `subscribe_tasks`**

```rust
fn subscribe_tasks<'a>(&'a self, queue_names: &'a [&'a str])
    -> impl futures_core::Stream<Item = String> + Send + 'a
{
    let db = self.db.clone();
    let queues: Vec<String> = queue_names.iter().map(|s| s.to_string()).collect();
    async_stream::stream! {
        let result = db
            .query("LIVE SELECT id FROM activity WHERE task_queue IN $qs AND status = 'pending'")
            .bind(("qs", queues))
            .await;
        // ... (same structure as subscribe_runnable)
    }
}
```

- [ ] **Step 4: Run the push test**

```bash
cargo test -p assay-workflow --test surreal_push \
    --features backend-surrealdb -- --nocapture
```

Expected: passes within 5s.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(workflow/surreal): subscribe_runnable + subscribe_tasks via LIVE SELECT"
```

---

### Task 3.17: Postgres `LISTEN/NOTIFY` (optional — skipped in Phase 1)

If Task 1.6 was skipped, land it here. Full detail:

- [ ] Add migration `crates/assay-workflow/migrations/postgres/<ts>_listen_notify.sql` with triggers
      on `workflow` and `activity` tables.

```sql
CREATE OR REPLACE FUNCTION assay_notify_runnable() RETURNS trigger AS $$
BEGIN
  IF NEW.status IN ('running','pending') AND (TG_OP = 'INSERT' OR OLD.status NOT IN ('running','pending')) THEN
    PERFORM pg_notify('assay_runnable_' || NEW.namespace, NEW.id);
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS workflow_runnable_notify ON workflow;
CREATE TRIGGER workflow_runnable_notify
  AFTER INSERT OR UPDATE OF status, next_dispatch_at ON workflow
  FOR EACH ROW EXECUTE FUNCTION assay_notify_runnable();

CREATE OR REPLACE FUNCTION assay_notify_task() RETURNS trigger AS $$
BEGIN
  PERFORM pg_notify('assay_task_' || NEW.task_queue, NEW.id::text);
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS workflow_task_notify ON activity;
CREATE TRIGGER workflow_task_notify
  AFTER INSERT ON activity
  FOR EACH ROW EXECUTE FUNCTION assay_notify_task();
```

- [ ] Implement the stream handler using `sqlx::postgres::PgListener` (same pattern as plan 10 Task
      1.4 notes):

```rust
fn subscribe_runnable(&self, namespace: &str)
    -> impl futures_core::Stream<Item = String> + Send + '_
{
    let pool = self.pool.clone();
    let ns = namespace.to_string();
    async_stream::stream! {
        let mut listener = match sqlx::postgres::PgListener::connect_with(&pool).await {
            Ok(l) => l,
            Err(e) => { tracing::error!(?e, "pg listener connect failed"); return; }
        };
        if listener.listen(&format!("assay_runnable_{ns}")).await.is_err() { return; }
        while let Ok(n) = listener.recv().await {
            yield n.payload().to_string();
        }
    }
}
```

- [ ] Integration test mirroring the Surreal push test — use testcontainers PG.

- [ ] Commit: `feat(workflow/pg): implement LISTEN/NOTIFY push streams`.

---

### Task 3.18: Full parametrised suite verification

- [ ] **Step 1: Run every integration test across all three backends**

```bash
cargo test -p assay-workflow --features "backend-postgres backend-sqlite backend-surrealdb" --test '*' -- --nocapture 2>&1 | tee /tmp/phase-3-suite.log
```

- [ ] **Step 2: Investigate any Surreal-only failures**

If a test passes on PG + SQLite but fails on Surreal, the contract has leaked. Fix in order:

1. Check if Surreal's semantic gap (no SERIALIZABLE) is the root cause → fix in the Surreal impl by
   adding optimistic retry loop.
2. Check if the test is PG-specific → move to a PG-only file with
   `#[cfg(feature = "backend-postgres")]`.
3. Check if the trait contract is under-specified → add a doc comment to the trait method and update
   the failing impl to match.

Never paper-over a failure with backend detection in the test — that defeats the
parametrised-testing invariant.

- [ ] **Step 3: Commit — milestone**

```bash
git commit --allow-empty -m "test(workflow): full parametrised suite green on PG + SQLite + Surreal"
```

---

### Phase 3 exit criteria

- `cargo test -p assay-workflow --features 'backend-postgres backend-sqlite backend-surrealdb'`
  green.
- Every `WorkflowStore` method has a Surreal impl (no `todo!()` remaining in `surrealdb.rs`).
- Push wake-up verified on PG (via LISTEN) and Surreal (via LIVE). SQLite returns empty by design.
- Scheduler runs a workflow end-to-end against SurrealDB via a new smoke test.
- Binary size of `cargo build --release -p assay-workflow --features backend-surrealdb` within 14 MB
  of PG+SQLite-only baseline (check with `cargo bloat`).

---

## What's next

**[12c](./12c-phase-4-6-auth-identity-zanzibar.md)** starts the auth stack: session, password, jwt,
biscuit, OIDC client, passkey, Zanzibar core. Phase 3 can run in parallel with 12c once Phase 2
(harness) lands — SurrealDB workflow backend and auth primitives don't touch the same files.
