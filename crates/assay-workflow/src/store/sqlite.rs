use anyhow::Result;
use sqlx::SqlitePool;

use crate::store::{ApiKeyRecord, NamespaceRecord, NamespaceStats, QueueStats, WorkflowStore};
use crate::types::*;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS namespaces (
    name            TEXT PRIMARY KEY,
    created_at      REAL NOT NULL
);

INSERT OR IGNORE INTO namespaces (name, created_at)
    VALUES ('main', strftime('%s', 'now'));

CREATE TABLE IF NOT EXISTS workflows (
    id              TEXT PRIMARY KEY,
    namespace       TEXT NOT NULL DEFAULT 'main',
    run_id          TEXT NOT NULL,
    workflow_type   TEXT NOT NULL,
    task_queue      TEXT NOT NULL DEFAULT 'main',
    status          TEXT NOT NULL DEFAULT 'PENDING',
    input           TEXT,
    result          TEXT,
    error           TEXT,
    parent_id       TEXT,
    claimed_by      TEXT,
    created_at      REAL NOT NULL,
    updated_at      REAL NOT NULL,
    completed_at    REAL
);
CREATE INDEX IF NOT EXISTS idx_wf_status_queue ON workflows(status, task_queue);
CREATE INDEX IF NOT EXISTS idx_wf_namespace ON workflows(namespace);

CREATE TABLE IF NOT EXISTS workflow_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    workflow_id     TEXT NOT NULL REFERENCES workflows(id),
    seq             INTEGER NOT NULL,
    event_type      TEXT NOT NULL,
    payload         TEXT,
    timestamp       REAL NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_wf_events_lookup ON workflow_events(workflow_id, seq);

CREATE TABLE IF NOT EXISTS workflow_activities (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    workflow_id     TEXT NOT NULL REFERENCES workflows(id),
    seq             INTEGER NOT NULL,
    name            TEXT NOT NULL,
    task_queue      TEXT NOT NULL DEFAULT 'main',
    input           TEXT,
    status          TEXT NOT NULL DEFAULT 'PENDING',
    result          TEXT,
    error           TEXT,
    attempt         INTEGER NOT NULL DEFAULT 1,
    max_attempts    INTEGER NOT NULL DEFAULT 3,
    initial_interval_secs   REAL NOT NULL DEFAULT 1,
    backoff_coefficient     REAL NOT NULL DEFAULT 2,
    start_to_close_secs     REAL NOT NULL DEFAULT 300,
    heartbeat_timeout_secs  REAL,
    claimed_by      TEXT,
    scheduled_at    REAL NOT NULL,
    started_at      REAL,
    completed_at    REAL,
    last_heartbeat  REAL
);
CREATE INDEX IF NOT EXISTS idx_wf_act_pending ON workflow_activities(task_queue, status, scheduled_at);

CREATE TABLE IF NOT EXISTS workflow_timers (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    workflow_id     TEXT NOT NULL REFERENCES workflows(id),
    seq             INTEGER NOT NULL,
    fire_at         REAL NOT NULL,
    fired           INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_wf_timers_due ON workflow_timers(fire_at);

CREATE TABLE IF NOT EXISTS workflow_signals (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    workflow_id     TEXT NOT NULL REFERENCES workflows(id),
    name            TEXT NOT NULL,
    payload         TEXT,
    consumed        INTEGER NOT NULL DEFAULT 0,
    received_at     REAL NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_wf_signals_lookup ON workflow_signals(workflow_id, name, consumed);

CREATE TABLE IF NOT EXISTS workflow_schedules (
    name            TEXT NOT NULL,
    namespace       TEXT NOT NULL DEFAULT 'main',
    workflow_type   TEXT NOT NULL,
    cron_expr       TEXT NOT NULL,
    input           TEXT,
    task_queue      TEXT NOT NULL DEFAULT 'main',
    overlap_policy  TEXT NOT NULL DEFAULT 'skip',
    paused          INTEGER NOT NULL DEFAULT 0,
    last_run_at     REAL,
    next_run_at     REAL,
    last_workflow_id TEXT,
    created_at      REAL NOT NULL,
    PRIMARY KEY (namespace, name)
);

CREATE TABLE IF NOT EXISTS workflow_workers (
    id              TEXT PRIMARY KEY,
    namespace       TEXT NOT NULL DEFAULT 'main',
    identity        TEXT NOT NULL,
    task_queue      TEXT NOT NULL,
    workflows       TEXT,
    activities      TEXT,
    max_concurrent_workflows  INTEGER NOT NULL DEFAULT 10,
    max_concurrent_activities INTEGER NOT NULL DEFAULT 10,
    active_tasks    INTEGER NOT NULL DEFAULT 0,
    last_heartbeat  REAL NOT NULL,
    registered_at   REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS workflow_snapshots (
    workflow_id     TEXT NOT NULL REFERENCES workflows(id),
    event_seq       INTEGER NOT NULL,
    state_json      TEXT NOT NULL,
    created_at      REAL NOT NULL,
    PRIMARY KEY (workflow_id, event_seq)
);

CREATE TABLE IF NOT EXISTS api_keys (
    key_hash        TEXT PRIMARY KEY,
    prefix          TEXT NOT NULL,
    label           TEXT,
    created_at      REAL NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_api_keys_prefix ON api_keys(prefix);

CREATE TABLE IF NOT EXISTS engine_lock (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    instance_id     TEXT NOT NULL,
    started_at      REAL NOT NULL,
    last_heartbeat  REAL NOT NULL
);
"#;

/// Stale lock timeout — if the lock holder hasn't heartbeated in this
/// many seconds, assume it's dead and allow takeover.
const LOCK_STALE_SECS: f64 = 60.0;
/// How often to refresh the lock heartbeat.
const LOCK_HEARTBEAT_SECS: u64 = 15;

pub struct SqliteStore {
    pool: SqlitePool,
    instance_id: String,
}

impl SqliteStore {
    pub async fn new(url: &str) -> Result<Self> {
        let pool = SqlitePool::connect(url).await?;
        let instance_id = format!("assay-{:016x}", {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            std::time::SystemTime::now().hash(&mut h);
            std::process::id().hash(&mut h);
            h.finish()
        });
        let store = Self { pool, instance_id };
        store.migrate().await?;
        Ok(store)
    }

    /// Acquire the single-instance engine lock.
    /// Returns an error if another instance is already running.
    pub async fn acquire_engine_lock(&self) -> Result<()> {
        let now = timestamp_now();

        // Try to insert the lock
        let result = sqlx::query(
            "INSERT INTO engine_lock (id, instance_id, started_at, last_heartbeat) VALUES (1, ?, ?, ?)",
        )
        .bind(&self.instance_id)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(_) => {
                // Lock exists — check if it's stale
                let row: Option<(String, f64)> = sqlx::query_as(
                    "SELECT instance_id, last_heartbeat FROM engine_lock WHERE id = 1",
                )
                .fetch_optional(&self.pool)
                .await?;

                if let Some((existing_id, last_hb)) = row {
                    if now - last_hb > LOCK_STALE_SECS {
                        // Stale lock — take over
                        sqlx::query(
                            "UPDATE engine_lock SET instance_id = ?, started_at = ?, last_heartbeat = ? WHERE id = 1",
                        )
                        .bind(&self.instance_id)
                        .bind(now)
                        .bind(now)
                        .execute(&self.pool)
                        .await?;
                        tracing::warn!(
                            "Took over stale engine lock from {existing_id} (last heartbeat {:.0}s ago)",
                            now - last_hb
                        );
                        Ok(())
                    } else {
                        let age = now - last_hb;
                        anyhow::bail!(
                            "Another assay engine instance is already running (id: {existing_id}, \
                             last heartbeat {age:.0}s ago).\n\n\
                             SQLite only supports a single engine instance. For multi-instance \
                             deployment (Kubernetes, Docker Swarm), use PostgreSQL:\n\n\
                             \x20 assay serve --backend postgres://user:pass@host:5432/dbname"
                        );
                    }
                } else {
                    anyhow::bail!("Unexpected engine lock state");
                }
            }
        }
    }

    /// Refresh the engine lock heartbeat. Called periodically by the engine.
    pub async fn refresh_engine_lock(&self) -> Result<()> {
        sqlx::query("UPDATE engine_lock SET last_heartbeat = ? WHERE id = 1 AND instance_id = ?")
            .bind(timestamp_now())
            .bind(&self.instance_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Release the engine lock on shutdown.
    pub async fn release_engine_lock(&self) -> Result<()> {
        sqlx::query("DELETE FROM engine_lock WHERE id = 1 AND instance_id = ?")
            .bind(&self.instance_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Start background task to keep the lock alive.
    pub fn spawn_lock_heartbeat(self: &std::sync::Arc<Self>) {
        let store = std::sync::Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(LOCK_HEARTBEAT_SECS));
            loop {
                tick.tick().await;
                if let Err(e) = store.refresh_engine_lock().await {
                    tracing::error!("Engine lock heartbeat failed: {e}");
                }
            }
        });
    }

    async fn migrate(&self) -> Result<()> {
        for statement in SCHEMA.split(';') {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed).execute(&self.pool).await?;
            }
        }
        Ok(())
    }
}

impl WorkflowStore for SqliteStore {
    // ── Namespaces ─────────────────────────────────────────

    async fn create_namespace(&self, name: &str) -> Result<()> {
        sqlx::query("INSERT INTO namespaces (name, created_at) VALUES (?, ?)")
            .bind(name)
            .bind(timestamp_now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_namespaces(&self) -> Result<Vec<NamespaceRecord>> {
        let rows = sqlx::query_as::<_, (String, f64)>(
            "SELECT name, created_at FROM namespaces ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(name, created_at)| NamespaceRecord { name, created_at })
            .collect())
    }

    async fn delete_namespace(&self, name: &str) -> Result<bool> {
        let res = sqlx::query("DELETE FROM namespaces WHERE name = ?")
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn get_namespace_stats(&self, namespace: &str) -> Result<NamespaceStats> {
        let total: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM workflows WHERE namespace = ?")
                .bind(namespace)
                .fetch_one(&self.pool)
                .await?;
        let running: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM workflows WHERE namespace = ? AND status = 'RUNNING'",
        )
        .bind(namespace)
        .fetch_one(&self.pool)
        .await?;
        let pending: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM workflows WHERE namespace = ? AND status = 'PENDING'",
        )
        .bind(namespace)
        .fetch_one(&self.pool)
        .await?;
        let completed: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM workflows WHERE namespace = ? AND status = 'COMPLETED'",
        )
        .bind(namespace)
        .fetch_one(&self.pool)
        .await?;
        let failed: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM workflows WHERE namespace = ? AND status = 'FAILED'",
        )
        .bind(namespace)
        .fetch_one(&self.pool)
        .await?;
        let schedules: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM workflow_schedules WHERE namespace = ?")
                .bind(namespace)
                .fetch_one(&self.pool)
                .await?;
        let workers: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM workflow_workers WHERE namespace = ?")
                .bind(namespace)
                .fetch_one(&self.pool)
                .await?;

        Ok(NamespaceStats {
            namespace: namespace.to_string(),
            total_workflows: total.0,
            running: running.0,
            pending: pending.0,
            completed: completed.0,
            failed: failed.0,
            schedules: schedules.0,
            workers: workers.0,
        })
    }

    // ── Workflows ──────────────────────────────────────────

    async fn create_workflow(&self, wf: &WorkflowRecord) -> Result<()> {
        sqlx::query(
            "INSERT INTO workflows (id, namespace, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, created_at, updated_at, completed_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&wf.id)
        .bind(&wf.namespace)
        .bind(&wf.run_id)
        .bind(&wf.workflow_type)
        .bind(&wf.task_queue)
        .bind(&wf.status)
        .bind(&wf.input)
        .bind(&wf.result)
        .bind(&wf.error)
        .bind(&wf.parent_id)
        .bind(&wf.claimed_by)
        .bind(wf.created_at)
        .bind(wf.updated_at)
        .bind(wf.completed_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_workflow(&self, id: &str) -> Result<Option<WorkflowRecord>> {
        let row = sqlx::query_as::<_, SqliteWorkflowRow>(
            "SELECT id, namespace, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, created_at, updated_at, completed_at FROM workflows WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    async fn list_workflows(
        &self,
        namespace: &str,
        status: Option<WorkflowStatus>,
        workflow_type: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<WorkflowRecord>> {
        let status_str = status.map(|s| s.to_string());
        let rows = sqlx::query_as::<_, SqliteWorkflowRow>(
            "SELECT id, namespace, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, created_at, updated_at, completed_at
             FROM workflows
             WHERE namespace = ?
               AND (? IS NULL OR status = ?)
               AND (? IS NULL OR workflow_type = ?)
             ORDER BY created_at DESC
             LIMIT ? OFFSET ?",
        )
        .bind(namespace)
        .bind(&status_str)
        .bind(&status_str)
        .bind(workflow_type)
        .bind(workflow_type)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update_workflow_status(
        &self,
        id: &str,
        status: WorkflowStatus,
        result: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        let now = timestamp_now();
        let completed_at = if status.is_terminal() { Some(now) } else { None };
        sqlx::query(
            "UPDATE workflows SET status = ?, result = COALESCE(?, result), error = COALESCE(?, error), updated_at = ?, completed_at = COALESCE(?, completed_at) WHERE id = ?",
        )
        .bind(status.to_string())
        .bind(result)
        .bind(error)
        .bind(now)
        .bind(completed_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn claim_workflow(&self, id: &str, worker_id: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE workflows SET claimed_by = ?, status = 'RUNNING', updated_at = ? WHERE id = ? AND claimed_by IS NULL",
        )
        .bind(worker_id)
        .bind(timestamp_now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    // ── Events ─────────────────────────────────────────────

    async fn append_event(&self, ev: &WorkflowEvent) -> Result<i64> {
        let res = sqlx::query(
            "INSERT INTO workflow_events (workflow_id, seq, event_type, payload, timestamp) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&ev.workflow_id)
        .bind(ev.seq)
        .bind(&ev.event_type)
        .bind(&ev.payload)
        .bind(ev.timestamp)
        .execute(&self.pool)
        .await?;
        Ok(res.last_insert_rowid())
    }

    async fn list_events(&self, workflow_id: &str) -> Result<Vec<WorkflowEvent>> {
        let rows = sqlx::query_as::<_, SqliteEventRow>(
            "SELECT id, workflow_id, seq, event_type, payload, timestamp FROM workflow_events WHERE workflow_id = ? ORDER BY seq ASC",
        )
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get_event_count(&self, workflow_id: &str) -> Result<i64> {
        let row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM workflow_events WHERE workflow_id = ?")
                .bind(workflow_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(row.0)
    }

    // ── Activities ──────────────────────────────────────────

    async fn create_activity(&self, act: &WorkflowActivity) -> Result<i64> {
        let res = sqlx::query(
            "INSERT INTO workflow_activities (workflow_id, seq, name, task_queue, input, status, attempt, max_attempts, initial_interval_secs, backoff_coefficient, start_to_close_secs, heartbeat_timeout_secs, scheduled_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&act.workflow_id)
        .bind(act.seq)
        .bind(&act.name)
        .bind(&act.task_queue)
        .bind(&act.input)
        .bind(&act.status)
        .bind(act.attempt)
        .bind(act.max_attempts)
        .bind(act.initial_interval_secs)
        .bind(act.backoff_coefficient)
        .bind(act.start_to_close_secs)
        .bind(act.heartbeat_timeout_secs)
        .bind(act.scheduled_at)
        .execute(&self.pool)
        .await?;
        Ok(res.last_insert_rowid())
    }

    async fn claim_activity(
        &self,
        task_queue: &str,
        worker_id: &str,
    ) -> Result<Option<WorkflowActivity>> {
        let now = timestamp_now();
        let row = sqlx::query_as::<_, SqliteActivityRow>(
            "UPDATE workflow_activities SET status = 'RUNNING', claimed_by = ?, started_at = ?
             WHERE id = (
                SELECT id FROM workflow_activities
                WHERE task_queue = ? AND status = 'PENDING'
                ORDER BY scheduled_at ASC
                LIMIT 1
             )
             RETURNING id, workflow_id, seq, name, task_queue, input, status, result, error, attempt, max_attempts, initial_interval_secs, backoff_coefficient, start_to_close_secs, heartbeat_timeout_secs, claimed_by, scheduled_at, started_at, completed_at, last_heartbeat",
        )
        .bind(worker_id)
        .bind(now)
        .bind(task_queue)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    async fn complete_activity(
        &self,
        id: i64,
        result: Option<&str>,
        error: Option<&str>,
        failed: bool,
    ) -> Result<()> {
        let status = if failed { "FAILED" } else { "COMPLETED" };
        sqlx::query(
            "UPDATE workflow_activities SET status = ?, result = ?, error = ?, completed_at = ? WHERE id = ?",
        )
        .bind(status)
        .bind(result)
        .bind(error)
        .bind(timestamp_now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn heartbeat_activity(&self, id: i64, _details: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE workflow_activities SET last_heartbeat = ? WHERE id = ?")
            .bind(timestamp_now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_timed_out_activities(&self, now: f64) -> Result<Vec<WorkflowActivity>> {
        let rows = sqlx::query_as::<_, SqliteActivityRow>(
            "SELECT id, workflow_id, seq, name, task_queue, input, status, result, error, attempt, max_attempts, initial_interval_secs, backoff_coefficient, start_to_close_secs, heartbeat_timeout_secs, claimed_by, scheduled_at, started_at, completed_at, last_heartbeat
             FROM workflow_activities
             WHERE status = 'RUNNING'
               AND heartbeat_timeout_secs IS NOT NULL
               AND last_heartbeat IS NOT NULL
               AND (? - last_heartbeat) > heartbeat_timeout_secs",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    // ── Timers ──────────────────────────────────────────────

    async fn create_timer(&self, timer: &WorkflowTimer) -> Result<i64> {
        let res = sqlx::query(
            "INSERT INTO workflow_timers (workflow_id, seq, fire_at, fired) VALUES (?, ?, ?, 0)",
        )
        .bind(&timer.workflow_id)
        .bind(timer.seq)
        .bind(timer.fire_at)
        .execute(&self.pool)
        .await?;
        Ok(res.last_insert_rowid())
    }

    async fn fire_due_timers(&self, now: f64) -> Result<Vec<WorkflowTimer>> {
        let rows = sqlx::query_as::<_, SqliteTimerRow>(
            "UPDATE workflow_timers SET fired = 1
             WHERE fired = 0 AND fire_at <= ?
             RETURNING id, workflow_id, seq, fire_at, fired",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    // ── Signals ─────────────────────────────────────────────

    async fn send_signal(&self, sig: &WorkflowSignal) -> Result<i64> {
        let res = sqlx::query(
            "INSERT INTO workflow_signals (workflow_id, name, payload, consumed, received_at) VALUES (?, ?, ?, 0, ?)",
        )
        .bind(&sig.workflow_id)
        .bind(&sig.name)
        .bind(&sig.payload)
        .bind(sig.received_at)
        .execute(&self.pool)
        .await?;
        Ok(res.last_insert_rowid())
    }

    async fn consume_signals(
        &self,
        workflow_id: &str,
        name: &str,
    ) -> Result<Vec<WorkflowSignal>> {
        let rows = sqlx::query_as::<_, SqliteSignalRow>(
            "UPDATE workflow_signals SET consumed = 1
             WHERE workflow_id = ? AND name = ? AND consumed = 0
             RETURNING id, workflow_id, name, payload, consumed, received_at",
        )
        .bind(workflow_id)
        .bind(name)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    // ── Schedules ───────────────────────────────────────────

    async fn create_schedule(&self, sched: &WorkflowSchedule) -> Result<()> {
        sqlx::query(
            "INSERT INTO workflow_schedules (name, namespace, workflow_type, cron_expr, input, task_queue, overlap_policy, paused, last_run_at, next_run_at, last_workflow_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&sched.name)
        .bind(&sched.namespace)
        .bind(&sched.workflow_type)
        .bind(&sched.cron_expr)
        .bind(&sched.input)
        .bind(&sched.task_queue)
        .bind(&sched.overlap_policy)
        .bind(sched.paused)
        .bind(sched.last_run_at)
        .bind(sched.next_run_at)
        .bind(&sched.last_workflow_id)
        .bind(sched.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_schedule(&self, namespace: &str, name: &str) -> Result<Option<WorkflowSchedule>> {
        let row = sqlx::query_as::<_, SqliteScheduleRow>(
            "SELECT name, namespace, workflow_type, cron_expr, input, task_queue, overlap_policy, paused, last_run_at, next_run_at, last_workflow_id, created_at
             FROM workflow_schedules WHERE namespace = ? AND name = ?",
        )
        .bind(namespace)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    async fn list_schedules(&self, namespace: &str) -> Result<Vec<WorkflowSchedule>> {
        let rows = sqlx::query_as::<_, SqliteScheduleRow>(
            "SELECT name, namespace, workflow_type, cron_expr, input, task_queue, overlap_policy, paused, last_run_at, next_run_at, last_workflow_id, created_at
             FROM workflow_schedules WHERE namespace = ? ORDER BY name",
        )
        .bind(namespace)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update_schedule_last_run(
        &self,
        namespace: &str,
        name: &str,
        last_run_at: f64,
        next_run_at: f64,
        workflow_id: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE workflow_schedules SET last_run_at = ?, next_run_at = ?, last_workflow_id = ? WHERE namespace = ? AND name = ?",
        )
        .bind(last_run_at)
        .bind(next_run_at)
        .bind(workflow_id)
        .bind(namespace)
        .bind(name)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_schedule(&self, namespace: &str, name: &str) -> Result<bool> {
        let res =
            sqlx::query("DELETE FROM workflow_schedules WHERE namespace = ? AND name = ?")
                .bind(namespace)
                .bind(name)
                .execute(&self.pool)
                .await?;
        Ok(res.rows_affected() > 0)
    }

    // ── Workers ─────────────────────────────────────────────

    async fn register_worker(&self, w: &WorkflowWorker) -> Result<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO workflow_workers (id, namespace, identity, task_queue, workflows, activities, max_concurrent_workflows, max_concurrent_activities, active_tasks, last_heartbeat, registered_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&w.id)
        .bind(&w.namespace)
        .bind(&w.identity)
        .bind(&w.task_queue)
        .bind(&w.workflows)
        .bind(&w.activities)
        .bind(w.max_concurrent_workflows)
        .bind(w.max_concurrent_activities)
        .bind(w.active_tasks)
        .bind(w.last_heartbeat)
        .bind(w.registered_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn heartbeat_worker(&self, id: &str, now: f64) -> Result<()> {
        sqlx::query("UPDATE workflow_workers SET last_heartbeat = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_workers(&self, namespace: &str) -> Result<Vec<WorkflowWorker>> {
        let rows = sqlx::query_as::<_, SqliteWorkerRow>(
            "SELECT id, namespace, identity, task_queue, workflows, activities, max_concurrent_workflows, max_concurrent_activities, active_tasks, last_heartbeat, registered_at
             FROM workflow_workers WHERE namespace = ? ORDER BY registered_at",
        )
        .bind(namespace)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn remove_dead_workers(&self, cutoff: f64) -> Result<Vec<String>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT id FROM workflow_workers WHERE last_heartbeat < ?")
                .bind(cutoff)
                .fetch_all(&self.pool)
                .await?;
        let ids: Vec<String> = rows.into_iter().map(|r| r.0).collect();
        if !ids.is_empty() {
            sqlx::query("DELETE FROM workflow_workers WHERE last_heartbeat < ?")
                .bind(cutoff)
                .execute(&self.pool)
                .await?;
        }
        Ok(ids)
    }

    // ── API Keys ────────────────────────────────────────────

    async fn create_api_key(
        &self,
        key_hash: &str,
        prefix: &str,
        label: Option<&str>,
        created_at: f64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO api_keys (key_hash, prefix, label, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(key_hash)
        .bind(prefix)
        .bind(label)
        .bind(created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn validate_api_key(&self, key_hash: &str) -> Result<bool> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM api_keys WHERE key_hash = ?")
                .bind(key_hash)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.is_some())
    }

    async fn list_api_keys(&self) -> Result<Vec<ApiKeyRecord>> {
        let rows = sqlx::query_as::<_, (String, Option<String>, f64)>(
            "SELECT prefix, label, created_at FROM api_keys ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(prefix, label, created_at)| ApiKeyRecord {
                prefix,
                label,
                created_at,
            })
            .collect())
    }

    async fn revoke_api_key(&self, prefix: &str) -> Result<bool> {
        let res = sqlx::query("DELETE FROM api_keys WHERE prefix = ?")
            .bind(prefix)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    // ── Child Workflows ─────────────────────────────────────

    async fn list_child_workflows(&self, parent_id: &str) -> Result<Vec<WorkflowRecord>> {
        let rows = sqlx::query_as::<_, SqliteWorkflowRow>(
            "SELECT id, namespace, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, created_at, updated_at, completed_at
             FROM workflows WHERE parent_id = ? ORDER BY created_at ASC",
        )
        .bind(parent_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    // ── Snapshots ───────────────────────────────────────────

    async fn create_snapshot(
        &self,
        workflow_id: &str,
        event_seq: i32,
        state_json: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO workflow_snapshots (workflow_id, event_seq, state_json, created_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(workflow_id)
        .bind(event_seq)
        .bind(state_json)
        .bind(timestamp_now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_latest_snapshot(
        &self,
        workflow_id: &str,
    ) -> Result<Option<WorkflowSnapshot>> {
        let row = sqlx::query_as::<_, (String, i32, String, f64)>(
            "SELECT workflow_id, event_seq, state_json, created_at
             FROM workflow_snapshots WHERE workflow_id = ?
             ORDER BY event_seq DESC LIMIT 1",
        )
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(workflow_id, event_seq, state_json, created_at)| WorkflowSnapshot {
            workflow_id,
            event_seq,
            state_json,
            created_at,
        }))
    }

    // ── Queue Stats ─────────────────────────────────────────

    async fn get_queue_stats(&self, namespace: &str) -> Result<Vec<QueueStats>> {
        // Gather activity stats per queue for workflows in this namespace
        let rows = sqlx::query_as::<_, (String, i64, i64)>(
            "SELECT a.task_queue,
                    SUM(CASE WHEN a.status = 'PENDING' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN a.status = 'RUNNING' THEN 1 ELSE 0 END)
             FROM workflow_activities a
             INNER JOIN workflows w ON w.id = a.workflow_id
             WHERE w.namespace = ?
             GROUP BY a.task_queue",
        )
        .bind(namespace)
        .fetch_all(&self.pool)
        .await?;

        let mut stats: Vec<QueueStats> = rows
            .into_iter()
            .map(|(queue, pending, running)| QueueStats {
                queue,
                pending_activities: pending,
                running_activities: running,
                workers: 0,
            })
            .collect();

        // Gather worker counts per queue in this namespace
        let worker_rows = sqlx::query_as::<_, (String, i64)>(
            "SELECT task_queue, COUNT(*) FROM workflow_workers WHERE namespace = ? GROUP BY task_queue",
        )
        .bind(namespace)
        .fetch_all(&self.pool)
        .await?;

        for (queue, count) in worker_rows {
            if let Some(s) = stats.iter_mut().find(|s| s.queue == queue) {
                s.workers = count;
            } else {
                stats.push(QueueStats {
                    queue,
                    pending_activities: 0,
                    running_activities: 0,
                    workers: count,
                });
            }
        }

        stats.sort_by(|a, b| a.queue.cmp(&b.queue));
        Ok(stats)
    }

    // ── Leader Election ─────────────────────────────────────

    async fn try_acquire_scheduler_lock(&self) -> Result<bool> {
        // SQLite is single-instance — always the leader.
        // Also refresh the engine lock heartbeat on each scheduler tick.
        self.refresh_engine_lock().await.ok();
        Ok(true)
    }
}

fn timestamp_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

// ── SQLite row types (sqlx::FromRow) ────────────────────────

#[derive(sqlx::FromRow)]
struct SqliteWorkflowRow {
    id: String,
    namespace: String,
    run_id: String,
    workflow_type: String,
    task_queue: String,
    status: String,
    input: Option<String>,
    result: Option<String>,
    error: Option<String>,
    parent_id: Option<String>,
    claimed_by: Option<String>,
    created_at: f64,
    updated_at: f64,
    completed_at: Option<f64>,
}

impl From<SqliteWorkflowRow> for WorkflowRecord {
    fn from(r: SqliteWorkflowRow) -> Self {
        Self {
            id: r.id,
            namespace: r.namespace,
            run_id: r.run_id,
            workflow_type: r.workflow_type,
            task_queue: r.task_queue,
            status: r.status,
            input: r.input,
            result: r.result,
            error: r.error,
            parent_id: r.parent_id,
            claimed_by: r.claimed_by,
            created_at: r.created_at,
            updated_at: r.updated_at,
            completed_at: r.completed_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct SqliteEventRow {
    id: i64,
    workflow_id: String,
    seq: i32,
    event_type: String,
    payload: Option<String>,
    timestamp: f64,
}

impl From<SqliteEventRow> for WorkflowEvent {
    fn from(r: SqliteEventRow) -> Self {
        Self {
            id: Some(r.id),
            workflow_id: r.workflow_id,
            seq: r.seq,
            event_type: r.event_type,
            payload: r.payload,
            timestamp: r.timestamp,
        }
    }
}

#[derive(sqlx::FromRow)]
struct SqliteActivityRow {
    id: i64,
    workflow_id: String,
    seq: i32,
    name: String,
    task_queue: String,
    input: Option<String>,
    status: String,
    result: Option<String>,
    error: Option<String>,
    attempt: i32,
    max_attempts: i32,
    initial_interval_secs: f64,
    backoff_coefficient: f64,
    start_to_close_secs: f64,
    heartbeat_timeout_secs: Option<f64>,
    claimed_by: Option<String>,
    scheduled_at: f64,
    started_at: Option<f64>,
    completed_at: Option<f64>,
    last_heartbeat: Option<f64>,
}

impl From<SqliteActivityRow> for WorkflowActivity {
    fn from(r: SqliteActivityRow) -> Self {
        Self {
            id: Some(r.id),
            workflow_id: r.workflow_id,
            seq: r.seq,
            name: r.name,
            task_queue: r.task_queue,
            input: r.input,
            status: r.status,
            result: r.result,
            error: r.error,
            attempt: r.attempt,
            max_attempts: r.max_attempts,
            initial_interval_secs: r.initial_interval_secs,
            backoff_coefficient: r.backoff_coefficient,
            start_to_close_secs: r.start_to_close_secs,
            heartbeat_timeout_secs: r.heartbeat_timeout_secs,
            claimed_by: r.claimed_by,
            scheduled_at: r.scheduled_at,
            started_at: r.started_at,
            completed_at: r.completed_at,
            last_heartbeat: r.last_heartbeat,
        }
    }
}

#[derive(sqlx::FromRow)]
struct SqliteTimerRow {
    id: i64,
    workflow_id: String,
    seq: i32,
    fire_at: f64,
    fired: bool,
}

impl From<SqliteTimerRow> for WorkflowTimer {
    fn from(r: SqliteTimerRow) -> Self {
        Self {
            id: Some(r.id),
            workflow_id: r.workflow_id,
            seq: r.seq,
            fire_at: r.fire_at,
            fired: r.fired,
        }
    }
}

#[derive(sqlx::FromRow)]
struct SqliteSignalRow {
    id: i64,
    workflow_id: String,
    name: String,
    payload: Option<String>,
    consumed: bool,
    received_at: f64,
}

impl From<SqliteSignalRow> for WorkflowSignal {
    fn from(r: SqliteSignalRow) -> Self {
        Self {
            id: Some(r.id),
            workflow_id: r.workflow_id,
            name: r.name,
            payload: r.payload,
            consumed: r.consumed,
            received_at: r.received_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct SqliteScheduleRow {
    name: String,
    namespace: String,
    workflow_type: String,
    cron_expr: String,
    input: Option<String>,
    task_queue: String,
    overlap_policy: String,
    paused: bool,
    last_run_at: Option<f64>,
    next_run_at: Option<f64>,
    last_workflow_id: Option<String>,
    created_at: f64,
}

impl From<SqliteScheduleRow> for WorkflowSchedule {
    fn from(r: SqliteScheduleRow) -> Self {
        Self {
            name: r.name,
            namespace: r.namespace,
            workflow_type: r.workflow_type,
            cron_expr: r.cron_expr,
            input: r.input,
            task_queue: r.task_queue,
            overlap_policy: r.overlap_policy,
            paused: r.paused,
            last_run_at: r.last_run_at,
            next_run_at: r.next_run_at,
            last_workflow_id: r.last_workflow_id,
            created_at: r.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct SqliteWorkerRow {
    id: String,
    namespace: String,
    identity: String,
    task_queue: String,
    workflows: Option<String>,
    activities: Option<String>,
    max_concurrent_workflows: i32,
    max_concurrent_activities: i32,
    active_tasks: i32,
    last_heartbeat: f64,
    registered_at: f64,
}

impl From<SqliteWorkerRow> for WorkflowWorker {
    fn from(r: SqliteWorkerRow) -> Self {
        Self {
            id: r.id,
            namespace: r.namespace,
            identity: r.identity,
            task_queue: r.task_queue,
            workflows: r.workflows,
            activities: r.activities,
            max_concurrent_workflows: r.max_concurrent_workflows,
            max_concurrent_activities: r.max_concurrent_activities,
            active_tasks: r.active_tasks,
            last_heartbeat: r.last_heartbeat,
            registered_at: r.registered_at,
        }
    }
}
