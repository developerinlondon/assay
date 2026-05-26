//! Parametrised backend harness.
//!
//! Each test function is decorated with rstest cases for Postgres and SQLite.
//! The `Harness` enum wraps both concrete store types and delegates
//! `WorkflowStore` calls through explicit `match` arms, so test bodies remain
//! backend-agnostic without requiring `dyn Trait`.

// Some harness methods are only exercised by a subset of the tests
// (e.g. PG-only tests gated off on macOS). Silence dead_code rather
// than litter the file with per-method allows.
#![allow(dead_code)]
// Harness variant sizes differ substantially (PG testcontainer vs
// SQLite tempdir). Boxing wouldn't help the test ergonomics.
#![allow(clippy::large_enum_variant)]

use assay_domain::types::*;
use assay_domain::{NamespaceRecord, NamespaceStats, QueueStats};
use assay_workflow::WorkflowStore;
// Re-export types used by new harness methods so tests don't need extra imports.
pub use assay_domain::types::{SchedulePatch, WorkflowSchedule, WorkflowSnapshot, WorkflowWorker};

// ── Harness ───────────────────────────────────────────────────────────────────

pub enum Harness {
    #[cfg(feature = "backend-postgres")]
    Postgres {
        store: assay_workflow::PostgresStore,
        // Owns the per-test database and drops it when the harness leaves scope.
        database: TestPostgresDatabase,
    },
    #[cfg(feature = "backend-sqlite")]
    Sqlite {
        _tempdir: tempfile::TempDir,
        store: assay_workflow::SqliteStore,
    },
}

macro_rules! dispatch {
    ($self:expr, $store:ident => $body:expr) => {
        match $self {
            #[cfg(feature = "backend-postgres")]
            Self::Postgres { store: $store, .. } => $body,
            #[cfg(feature = "backend-sqlite")]
            Self::Sqlite { store: $store, .. } => $body,
        }
    };
}

impl Harness {
    pub async fn list_namespaces(&self) -> anyhow::Result<Vec<NamespaceRecord>> {
        dispatch!(self, s => s.list_namespaces().await)
    }

    pub async fn create_namespace(&self, name: &str) -> anyhow::Result<()> {
        dispatch!(self, s => s.create_namespace(name).await)
    }

    pub async fn delete_namespace(&self, name: &str) -> anyhow::Result<bool> {
        dispatch!(self, s => s.delete_namespace(name).await)
    }

    pub async fn get_namespace_stats(&self, ns: &str) -> anyhow::Result<NamespaceStats> {
        dispatch!(self, s => s.get_namespace_stats(ns).await)
    }

    pub async fn create_workflow(&self, wf: &WorkflowRecord) -> anyhow::Result<()> {
        dispatch!(self, s => s.create_workflow(wf).await)
    }

    pub async fn get_workflow(&self, id: &str) -> anyhow::Result<Option<WorkflowRecord>> {
        dispatch!(self, s => s.get_workflow(id).await)
    }

    pub async fn list_workflows(
        &self,
        namespace: &str,
        status: Option<WorkflowStatus>,
        workflow_type: Option<&str>,
        search_attrs_filter: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<Vec<WorkflowRecord>> {
        dispatch!(self, s => s.list_workflows(namespace, status, workflow_type, search_attrs_filter, limit, offset).await)
    }

    pub async fn update_workflow_status(
        &self,
        id: &str,
        status: WorkflowStatus,
        result: Option<&str>,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        dispatch!(self, s => s.update_workflow_status(id, status, result, error).await)
    }

    pub async fn claim_workflow(&self, id: &str, worker_id: &str) -> anyhow::Result<bool> {
        dispatch!(self, s => s.claim_workflow(id, worker_id).await)
    }

    pub async fn mark_workflow_dispatchable(&self, workflow_id: &str) -> anyhow::Result<()> {
        dispatch!(self, s => s.mark_workflow_dispatchable(workflow_id).await)
    }

    pub async fn claim_workflow_task(
        &self,
        task_queue: &str,
        worker_id: &str,
    ) -> anyhow::Result<Option<WorkflowRecord>> {
        dispatch!(self, s => s.claim_workflow_task(task_queue, worker_id).await)
    }

    pub async fn release_workflow_task(
        &self,
        workflow_id: &str,
        worker_id: &str,
    ) -> anyhow::Result<()> {
        dispatch!(self, s => s.release_workflow_task(workflow_id, worker_id).await)
    }

    pub async fn append_event(&self, ev: &WorkflowEvent) -> anyhow::Result<i64> {
        dispatch!(self, s => s.append_event(ev).await)
    }

    pub async fn list_events(&self, workflow_id: &str) -> anyhow::Result<Vec<WorkflowEvent>> {
        dispatch!(self, s => s.list_events(workflow_id).await)
    }

    pub async fn get_event_count(&self, workflow_id: &str) -> anyhow::Result<i64> {
        dispatch!(self, s => s.get_event_count(workflow_id).await)
    }

    pub async fn upsert_search_attributes(
        &self,
        workflow_id: &str,
        patch_json: &str,
    ) -> anyhow::Result<()> {
        dispatch!(self, s => s.upsert_search_attributes(workflow_id, patch_json).await)
    }

    pub async fn list_archivable_workflows(
        &self,
        cutoff: f64,
        limit: i64,
    ) -> anyhow::Result<Vec<WorkflowRecord>> {
        dispatch!(self, s => s.list_archivable_workflows(cutoff, limit).await)
    }

    pub async fn mark_archived_and_purge(
        &self,
        workflow_id: &str,
        archive_uri: &str,
        archived_at: f64,
    ) -> anyhow::Result<()> {
        dispatch!(self, s => s.mark_archived_and_purge(workflow_id, archive_uri, archived_at).await)
    }

    // ── Activities ────────────────────────────────────────────────────────────

    pub async fn create_activity(&self, act: &WorkflowActivity) -> anyhow::Result<i64> {
        dispatch!(self, s => s.create_activity(act).await)
    }

    pub async fn get_activity(&self, id: i64) -> anyhow::Result<Option<WorkflowActivity>> {
        dispatch!(self, s => s.get_activity(id).await)
    }

    pub async fn get_activity_by_workflow_seq(
        &self,
        workflow_id: &str,
        seq: i32,
    ) -> anyhow::Result<Option<WorkflowActivity>> {
        dispatch!(self, s => s.get_activity_by_workflow_seq(workflow_id, seq).await)
    }

    pub async fn claim_activity(
        &self,
        task_queue: &str,
        worker_id: &str,
    ) -> anyhow::Result<Option<WorkflowActivity>> {
        dispatch!(self, s => s.claim_activity(task_queue, worker_id).await)
    }

    pub async fn requeue_activity_for_retry(
        &self,
        id: i64,
        next_attempt: i32,
        next_scheduled_at: f64,
    ) -> anyhow::Result<()> {
        dispatch!(self, s => s.requeue_activity_for_retry(id, next_attempt, next_scheduled_at).await)
    }

    pub async fn complete_activity(
        &self,
        id: i64,
        result: Option<&str>,
        error: Option<&str>,
        failed: bool,
    ) -> anyhow::Result<()> {
        dispatch!(self, s => s.complete_activity(id, result, error, failed).await)
    }

    pub async fn heartbeat_activity(&self, id: i64, details: Option<&str>) -> anyhow::Result<()> {
        dispatch!(self, s => s.heartbeat_activity(id, details).await)
    }

    pub async fn get_timed_out_activities(
        &self,
        now: f64,
    ) -> anyhow::Result<Vec<WorkflowActivity>> {
        dispatch!(self, s => s.get_timed_out_activities(now).await)
    }

    pub async fn cancel_pending_activities(&self, workflow_id: &str) -> anyhow::Result<u64> {
        dispatch!(self, s => s.cancel_pending_activities(workflow_id).await)
    }

    // ── Timers ────────────────────────────────────────────────────────────────

    pub async fn create_timer(&self, timer: &WorkflowTimer) -> anyhow::Result<i64> {
        dispatch!(self, s => s.create_timer(timer).await)
    }

    pub async fn get_timer_by_workflow_seq(
        &self,
        workflow_id: &str,
        seq: i32,
    ) -> anyhow::Result<Option<WorkflowTimer>> {
        dispatch!(self, s => s.get_timer_by_workflow_seq(workflow_id, seq).await)
    }

    pub async fn fire_due_timers(&self, now: f64) -> anyhow::Result<Vec<WorkflowTimer>> {
        dispatch!(self, s => s.fire_due_timers(now).await)
    }

    pub async fn cancel_pending_timers(&self, workflow_id: &str) -> anyhow::Result<u64> {
        dispatch!(self, s => s.cancel_pending_timers(workflow_id).await)
    }

    // ── Signals ───────────────────────────────────────────────────────────────

    pub async fn send_signal(&self, signal: &WorkflowSignal) -> anyhow::Result<i64> {
        dispatch!(self, s => s.send_signal(signal).await)
    }

    pub async fn consume_signals(
        &self,
        workflow_id: &str,
        name: &str,
    ) -> anyhow::Result<Vec<WorkflowSignal>> {
        dispatch!(self, s => s.consume_signals(workflow_id, name).await)
    }

    // ── Schedules ─────────────────────────────────────────────────────────────

    pub async fn create_schedule(&self, sched: &WorkflowSchedule) -> anyhow::Result<()> {
        dispatch!(self, s => s.create_schedule(sched).await)
    }

    pub async fn get_schedule(
        &self,
        namespace: &str,
        name: &str,
    ) -> anyhow::Result<Option<WorkflowSchedule>> {
        dispatch!(self, s => s.get_schedule(namespace, name).await)
    }

    pub async fn list_schedules(&self, namespace: &str) -> anyhow::Result<Vec<WorkflowSchedule>> {
        dispatch!(self, s => s.list_schedules(namespace).await)
    }

    pub async fn update_schedule_last_run(
        &self,
        namespace: &str,
        name: &str,
        last_run_at: f64,
        next_run_at: f64,
        workflow_id: &str,
    ) -> anyhow::Result<()> {
        dispatch!(self, s => s.update_schedule_last_run(namespace, name, last_run_at, next_run_at, workflow_id).await)
    }

    pub async fn delete_schedule(&self, namespace: &str, name: &str) -> anyhow::Result<bool> {
        dispatch!(self, s => s.delete_schedule(namespace, name).await)
    }

    pub async fn update_schedule(
        &self,
        namespace: &str,
        name: &str,
        patch: &SchedulePatch,
    ) -> anyhow::Result<Option<WorkflowSchedule>> {
        dispatch!(self, s => s.update_schedule(namespace, name, patch).await)
    }

    pub async fn set_schedule_paused(
        &self,
        namespace: &str,
        name: &str,
        paused: bool,
    ) -> anyhow::Result<Option<WorkflowSchedule>> {
        dispatch!(self, s => s.set_schedule_paused(namespace, name, paused).await)
    }

    // ── Snapshots ─────────────────────────────────────────────────────────────

    pub async fn create_snapshot(
        &self,
        workflow_id: &str,
        event_seq: i32,
        state_json: &str,
    ) -> anyhow::Result<()> {
        dispatch!(self, s => s.create_snapshot(workflow_id, event_seq, state_json).await)
    }

    pub async fn get_latest_snapshot(
        &self,
        workflow_id: &str,
    ) -> anyhow::Result<Option<WorkflowSnapshot>> {
        dispatch!(self, s => s.get_latest_snapshot(workflow_id).await)
    }

    // ── Workers ───────────────────────────────────────────────────────────────

    pub async fn register_worker(&self, worker: &WorkflowWorker) -> anyhow::Result<()> {
        dispatch!(self, s => s.register_worker(worker).await)
    }

    pub async fn heartbeat_worker(&self, id: &str, now: f64) -> anyhow::Result<()> {
        dispatch!(self, s => s.heartbeat_worker(id, now).await)
    }

    pub async fn list_workers(&self, namespace: &str) -> anyhow::Result<Vec<WorkflowWorker>> {
        dispatch!(self, s => s.list_workers(namespace).await)
    }

    pub async fn remove_dead_workers(&self, cutoff: f64) -> anyhow::Result<Vec<String>> {
        dispatch!(self, s => s.remove_dead_workers(cutoff).await)
    }

    // ── Child Workflows ───────────────────────────────────────────────────────

    pub async fn list_child_workflows(
        &self,
        parent_id: &str,
    ) -> anyhow::Result<Vec<WorkflowRecord>> {
        dispatch!(self, s => s.list_child_workflows(parent_id).await)
    }

    // ── Queue Stats ───────────────────────────────────────────────────────────

    pub async fn get_queue_stats(&self, namespace: &str) -> anyhow::Result<Vec<QueueStats>> {
        dispatch!(self, s => s.get_queue_stats(namespace).await)
    }

    // ── Leader Election ───────────────────────────────────────────────────────

    pub async fn try_acquire_scheduler_lock(&self) -> anyhow::Result<bool> {
        dispatch!(self, s => s.try_acquire_scheduler_lock().await)
    }
}

// ── Backend selector ──────────────────────────────────────────────────────────

pub enum Backend {
    #[cfg(feature = "backend-postgres")]
    Postgres,
    #[cfg(feature = "backend-sqlite")]
    Sqlite,
}

impl Backend {
    pub async fn setup(self) -> anyhow::Result<Harness> {
        match self {
            #[cfg(feature = "backend-postgres")]
            Self::Postgres => postgres_harness().await,
            #[cfg(feature = "backend-sqlite")]
            Self::Sqlite => sqlite_harness().await,
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build a minimal valid `WorkflowRecord` for test use.
pub fn make_workflow(id: &str, namespace: &str, task_queue: &str) -> WorkflowRecord {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    WorkflowRecord {
        id: id.to_string(),
        namespace: namespace.to_string(),
        run_id: format!("run-{id}"),
        workflow_type: "test_wf".to_string(),
        task_queue: task_queue.to_string(),
        status: "PENDING".to_string(),
        input: Some(r#"{"key":"val"}"#.to_string()),
        result: None,
        error: None,
        parent_id: None,
        claimed_by: None,
        search_attributes: None,
        archived_at: None,
        archive_uri: None,
        created_at: now,
        updated_at: now,
        completed_at: None,
    }
}

/// Build a minimal valid `WorkflowEvent` for test use.
pub fn make_event(workflow_id: &str, seq: i32) -> WorkflowEvent {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    WorkflowEvent {
        id: None,
        workflow_id: workflow_id.to_string(),
        seq,
        event_type: "WorkflowStarted".to_string(),
        payload: Some(format!(r#"{{"seq":{seq}}}"#)),
        timestamp: now,
    }
}

// ── Per-backend setup ─────────────────────────────────────────────────────────

#[cfg(feature = "backend-postgres")]
async fn postgres_harness() -> anyhow::Result<Harness> {
    // CI points `TEST_DATABASE_URL` at a shared Postgres server and carves
    // out a fresh database per test. Local dev can fall back to an owned
    // testcontainer per Postgres case; the container is held by
    // `TestPostgresDatabase` and removed when the test database guard drops.
    // `env::var` returns `Ok("")` when the variable is set but empty — treat
    // that the same as unset so callers can "clear" the URL with
    // `TEST_DATABASE_URL=` without falling through to a broken connect.
    let server = TestPostgresServer::from_env_or_container().await?;
    let database = TestPostgresDatabase::create(server).await?;
    let store = assay_workflow::PostgresStore::from_pool(database.pool().clone()).await?;
    Ok(Harness::Postgres { store, database })
}

#[cfg(feature = "backend-postgres")]
pub struct TestPostgresDatabase {
    db_name: String,
    admin_opts: sqlx::postgres::PgConnectOptions,
    pool: sqlx::PgPool,
    server: TestPostgresServer,
}

#[cfg(feature = "backend-postgres")]
impl TestPostgresDatabase {
    pub async fn create(server: TestPostgresServer) -> anyhow::Result<Self> {
        use sqlx::postgres::{PgConnectOptions, PgPool};
        use std::str::FromStr;

        let admin_opts = PgConnectOptions::from_str(server.admin_url())?;
        let admin_pool = PgPool::connect_with(admin_opts.clone()).await?;

        let db_name = unique_db_name();
        sqlx::query(&format!(r#"CREATE DATABASE "{db_name}""#))
            .execute(&admin_pool)
            .await?;
        admin_pool.close().await;

        let pool = PgPool::connect_with(admin_opts.clone().database(&db_name)).await?;
        Ok(Self {
            db_name,
            admin_opts,
            pool,
            server,
        })
    }

    pub fn db_name(&self) -> &str {
        &self.db_name
    }

    pub fn pool(&self) -> &sqlx::PgPool {
        &self.pool
    }
}

#[cfg(feature = "backend-postgres")]
impl Drop for TestPostgresDatabase {
    fn drop(&mut self) {
        let db_name = self.db_name.clone();
        let admin_opts = self.admin_opts.clone();
        let cleanup = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(drop_test_postgres_database(admin_opts, db_name))
        })
        .join();

        match cleanup {
            Ok(Ok(())) => {}
            Ok(Err(err)) => eprintln!("failed to drop test Postgres database: {err:#}"),
            Err(_) => eprintln!("test Postgres database cleanup thread panicked"),
        }
    }
}

#[cfg(feature = "backend-postgres")]
async fn drop_test_postgres_database(
    admin_opts: sqlx::postgres::PgConnectOptions,
    db_name: String,
) -> anyhow::Result<()> {
    let admin_pool = sqlx::PgPool::connect_with(admin_opts).await?;
    sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1")
        .bind(&db_name)
        .execute(&admin_pool)
        .await?;
    sqlx::query(&format!(r#"DROP DATABASE IF EXISTS "{db_name}""#))
        .execute(&admin_pool)
        .await?;
    admin_pool.close().await;
    Ok(())
}

#[cfg(feature = "backend-postgres")]
pub struct TestPostgresServer {
    admin_url: String,
    container: Option<testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>>,
}

#[cfg(feature = "backend-postgres")]
impl TestPostgresServer {
    pub async fn from_env_or_container() -> anyhow::Result<Self> {
        if let Ok(url) = std::env::var("TEST_DATABASE_URL")
            && !url.is_empty()
        {
            return Ok(Self {
                admin_url: url,
                container: None,
            });
        }

        use testcontainers::ImageExt;
        use testcontainers::runners::AsyncRunner;
        use testcontainers_modules::postgres::Postgres as PgImage;

        let container = PgImage::default().with_tag("18-alpine").start().await?;
        let host = container.get_host().await?;
        let port = container.get_host_port_ipv4(5432).await?;
        Ok(Self {
            admin_url: format!("postgres://postgres:postgres@{host}:{port}/postgres"),
            container: Some(container),
        })
    }

    fn admin_url(&self) -> &str {
        &self.admin_url
    }
}

/// Produce a database name unique across tests within this process. Nanos + a
/// local atomic counter avoid collisions both across parallel test threads and
/// across runs that might rehydrate nanosecond-precision timestamps.
fn unique_db_name() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("assay_test_{t}_{n}")
}

#[cfg(feature = "backend-sqlite")]
async fn sqlite_harness() -> anyhow::Result<Harness> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("assay.db");
    let url = format!("sqlite://{}?mode=rwc", path.display());

    // The SQLite schema migration inserts "main" automatically (INSERT OR IGNORE);
    // no need to call create_namespace here.
    let store = assay_workflow::SqliteStore::new(&url).await?;

    Ok(Harness::Sqlite {
        _tempdir: dir,
        store,
    })
}
