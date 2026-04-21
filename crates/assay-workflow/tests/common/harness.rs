//! Parametrised backend harness.
//!
//! Each test function is decorated with rstest cases for Postgres, SQLite,
//! and SurrealDB. The `Harness` enum wraps all three concrete store types
//! and delegates `WorkflowStore` calls through explicit `match` arms, so
//! test bodies remain backend-agnostic without requiring `dyn Trait`.

use assay_core::types::*;
use assay_core::{NamespaceRecord, NamespaceStats};
use assay_workflow::WorkflowStore;

// ── Harness ───────────────────────────────────────────────────────────────────

pub enum Harness {
    #[cfg(feature = "backend-postgres")]
    Postgres {
        _container: testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
        store: assay_workflow::PostgresStore,
    },
    #[cfg(feature = "backend-sqlite")]
    Sqlite {
        _tempdir: tempfile::TempDir,
        store: assay_workflow::SqliteStore,
    },
    #[cfg(feature = "backend-surrealdb")]
    Surreal {
        _container: testcontainers::ContainerAsync<testcontainers_modules::surrealdb::SurrealDb>,
        store: assay_workflow::SurrealDbStore,
    },
}

macro_rules! dispatch {
    ($self:expr, $store:ident => $body:expr) => {
        match $self {
            #[cfg(feature = "backend-postgres")]
            Self::Postgres { store: $store, .. } => $body,
            #[cfg(feature = "backend-sqlite")]
            Self::Sqlite { store: $store, .. } => $body,
            #[cfg(feature = "backend-surrealdb")]
            Self::Surreal { store: $store, .. } => $body,
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

    pub async fn get_timed_out_activities(&self, now: f64) -> anyhow::Result<Vec<WorkflowActivity>> {
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
}

// ── Backend selector ──────────────────────────────────────────────────────────

pub enum Backend {
    #[cfg(feature = "backend-postgres")]
    Postgres,
    #[cfg(feature = "backend-sqlite")]
    Sqlite,
    #[cfg(feature = "backend-surrealdb")]
    Surreal,
}

impl Backend {
    pub async fn setup(self) -> anyhow::Result<Harness> {
        match self {
            #[cfg(feature = "backend-postgres")]
            Self::Postgres => postgres_harness().await,
            #[cfg(feature = "backend-sqlite")]
            Self::Sqlite => sqlite_harness().await,
            #[cfg(feature = "backend-surrealdb")]
            Self::Surreal => surreal_harness().await,
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
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres as PgImage;

    let container = PgImage::default().start().await?;
    let host = container.get_host().await?;
    let port = container.get_host_port_ipv4(5432).await?;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    // The PG schema migration inserts "main" automatically; no need to call
    // create_namespace here — it would fail with a unique-key violation.
    let store = assay_workflow::PostgresStore::new(&url).await?;

    Ok(Harness::Postgres {
        _container: container,
        store,
    })
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

#[cfg(feature = "backend-surrealdb")]
async fn surreal_harness() -> anyhow::Result<Harness> {
    use testcontainers::runners::AsyncRunner;
    use testcontainers::ImageExt;
    use testcontainers_modules::surrealdb::SurrealDb;

    // Pin to v3 — our surrealdb crate (3.x) speaks the v3 wire protocol.
    // testcontainers-modules 0.15 default image tag is v2.x, which causes
    // `Server sent no subprotocol` at handshake time.
    let container = SurrealDb::default()
        .with_tag("v3")
        .with_env_var("SURREAL_USER", "root")
        .with_env_var("SURREAL_PASS", "root")
        .start()
        .await?;
    let host = container.get_host().await?;
    let port = container.get_host_port_ipv4(8000).await?;
    let url = format!("ws://{host}:{port}");

    let store = assay_workflow::SurrealDbStore::connect_full(
        &url,
        "assay",
        "workflow",
        Some("root"),
        Some("root"),
    )
    .await?;

    Ok(Harness::Surreal {
        _container: container,
        store,
    })
}
