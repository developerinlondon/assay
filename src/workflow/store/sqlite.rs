use anyhow::Result;
use sqlx::SqlitePool;

use crate::workflow::store::WorkflowStore;
use crate::workflow::types::*;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS workflows (
    id              TEXT PRIMARY KEY,
    run_id          TEXT NOT NULL,
    workflow_type   TEXT NOT NULL,
    task_queue      TEXT NOT NULL DEFAULT 'default',
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
    task_queue      TEXT NOT NULL DEFAULT 'default',
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
    name            TEXT PRIMARY KEY,
    workflow_type   TEXT NOT NULL,
    cron_expr       TEXT NOT NULL,
    input           TEXT,
    task_queue      TEXT NOT NULL DEFAULT 'default',
    overlap_policy  TEXT NOT NULL DEFAULT 'skip',
    paused          INTEGER NOT NULL DEFAULT 0,
    last_run_at     REAL,
    next_run_at     REAL,
    last_workflow_id TEXT,
    created_at      REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS workflow_workers (
    id              TEXT PRIMARY KEY,
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
"#;

pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    pub async fn new(url: &str) -> Result<Self> {
        let pool = SqlitePool::connect(url).await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
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
    async fn create_workflow(&self, wf: &WorkflowRecord) -> Result<()> {
        sqlx::query(
            "INSERT INTO workflows (id, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, created_at, updated_at, completed_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&wf.id)
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
            "SELECT id, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, created_at, updated_at, completed_at FROM workflows WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    async fn list_workflows(
        &self,
        status: Option<WorkflowStatus>,
        workflow_type: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<WorkflowRecord>> {
        let status_str = status.map(|s| s.to_string());
        let rows = sqlx::query_as::<_, SqliteWorkflowRow>(
            "SELECT id, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, created_at, updated_at, completed_at
             FROM workflows
             WHERE (? IS NULL OR status = ?)
               AND (? IS NULL OR workflow_type = ?)
             ORDER BY created_at DESC
             LIMIT ? OFFSET ?",
        )
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
        // Atomic claim: find oldest pending activity on this queue and claim it
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
            "INSERT INTO workflow_schedules (name, workflow_type, cron_expr, input, task_queue, overlap_policy, paused, last_run_at, next_run_at, last_workflow_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&sched.name)
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

    async fn get_schedule(&self, name: &str) -> Result<Option<WorkflowSchedule>> {
        let row = sqlx::query_as::<_, SqliteScheduleRow>(
            "SELECT name, workflow_type, cron_expr, input, task_queue, overlap_policy, paused, last_run_at, next_run_at, last_workflow_id, created_at FROM workflow_schedules WHERE name = ?",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    async fn list_schedules(&self) -> Result<Vec<WorkflowSchedule>> {
        let rows = sqlx::query_as::<_, SqliteScheduleRow>(
            "SELECT name, workflow_type, cron_expr, input, task_queue, overlap_policy, paused, last_run_at, next_run_at, last_workflow_id, created_at FROM workflow_schedules ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update_schedule_last_run(
        &self,
        name: &str,
        last_run_at: f64,
        next_run_at: f64,
        workflow_id: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE workflow_schedules SET last_run_at = ?, next_run_at = ?, last_workflow_id = ? WHERE name = ?",
        )
        .bind(last_run_at)
        .bind(next_run_at)
        .bind(workflow_id)
        .bind(name)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_schedule(&self, name: &str) -> Result<bool> {
        let res = sqlx::query("DELETE FROM workflow_schedules WHERE name = ?")
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    // ── Workers ─────────────────────────────────────────────

    async fn register_worker(&self, w: &WorkflowWorker) -> Result<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO workflow_workers (id, identity, task_queue, workflows, activities, max_concurrent_workflows, max_concurrent_activities, active_tasks, last_heartbeat, registered_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&w.id)
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

    async fn list_workers(&self) -> Result<Vec<WorkflowWorker>> {
        let rows = sqlx::query_as::<_, SqliteWorkerRow>(
            "SELECT id, identity, task_queue, workflows, activities, max_concurrent_workflows, max_concurrent_activities, active_tasks, last_heartbeat, registered_at FROM workflow_workers ORDER BY registered_at",
        )
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
