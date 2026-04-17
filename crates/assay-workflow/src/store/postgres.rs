use anyhow::Result;
use sqlx::PgPool;

use crate::store::WorkflowStore;
use crate::types::*;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS namespaces (
    name            TEXT PRIMARY KEY,
    created_at      DOUBLE PRECISION NOT NULL
);
INSERT INTO namespaces (name, created_at)
    VALUES ('main', EXTRACT(EPOCH FROM NOW()))
    ON CONFLICT DO NOTHING;

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
    search_attributes TEXT,
    archived_at     DOUBLE PRECISION,
    archive_uri     TEXT,
    -- Workflow-task dispatch (Phase 9): see sqlite.rs for the full comment.
    needs_dispatch  BOOLEAN NOT NULL DEFAULT FALSE,
    dispatch_claimed_by    TEXT,
    dispatch_last_heartbeat DOUBLE PRECISION,
    created_at      DOUBLE PRECISION NOT NULL,
    updated_at      DOUBLE PRECISION NOT NULL,
    completed_at    DOUBLE PRECISION
);
CREATE INDEX IF NOT EXISTS idx_wf_status_queue ON workflows(status, task_queue);
CREATE INDEX IF NOT EXISTS idx_wf_namespace ON workflows(namespace);
CREATE INDEX IF NOT EXISTS idx_wf_dispatch ON workflows(task_queue, needs_dispatch, dispatch_claimed_by);

CREATE TABLE IF NOT EXISTS workflow_events (
    id              BIGSERIAL PRIMARY KEY,
    workflow_id     TEXT NOT NULL REFERENCES workflows(id),
    seq             INTEGER NOT NULL,
    event_type      TEXT NOT NULL,
    payload         TEXT,
    timestamp       DOUBLE PRECISION NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_wf_events_lookup ON workflow_events(workflow_id, seq);

CREATE TABLE IF NOT EXISTS workflow_activities (
    id              BIGSERIAL PRIMARY KEY,
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
    initial_interval_secs   DOUBLE PRECISION NOT NULL DEFAULT 1,
    backoff_coefficient     DOUBLE PRECISION NOT NULL DEFAULT 2,
    start_to_close_secs     DOUBLE PRECISION NOT NULL DEFAULT 300,
    heartbeat_timeout_secs  DOUBLE PRECISION,
    claimed_by      TEXT,
    scheduled_at    DOUBLE PRECISION NOT NULL,
    started_at      DOUBLE PRECISION,
    completed_at    DOUBLE PRECISION,
    last_heartbeat  DOUBLE PRECISION,
    UNIQUE (workflow_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_wf_act_pending ON workflow_activities(task_queue, status, scheduled_at);

CREATE TABLE IF NOT EXISTS workflow_timers (
    id              BIGSERIAL PRIMARY KEY,
    workflow_id     TEXT NOT NULL REFERENCES workflows(id),
    seq             INTEGER NOT NULL,
    fire_at         DOUBLE PRECISION NOT NULL,
    fired           BOOLEAN NOT NULL DEFAULT FALSE,
    UNIQUE (workflow_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_wf_timers_due ON workflow_timers(fire_at) WHERE fired = FALSE;

CREATE TABLE IF NOT EXISTS workflow_signals (
    id              BIGSERIAL PRIMARY KEY,
    workflow_id     TEXT NOT NULL REFERENCES workflows(id),
    name            TEXT NOT NULL,
    payload         TEXT,
    consumed        BOOLEAN NOT NULL DEFAULT FALSE,
    received_at     DOUBLE PRECISION NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_wf_signals_lookup ON workflow_signals(workflow_id, name, consumed);

CREATE TABLE IF NOT EXISTS workflow_schedules (
    namespace       TEXT NOT NULL DEFAULT 'main',
    name            TEXT NOT NULL,
    workflow_type   TEXT NOT NULL,
    cron_expr       TEXT NOT NULL,
    timezone        TEXT NOT NULL DEFAULT 'UTC',
    input           TEXT,
    task_queue      TEXT NOT NULL DEFAULT 'main',
    overlap_policy  TEXT NOT NULL DEFAULT 'skip',
    paused          BOOLEAN NOT NULL DEFAULT FALSE,
    last_run_at     DOUBLE PRECISION,
    next_run_at     DOUBLE PRECISION,
    last_workflow_id TEXT,
    created_at      DOUBLE PRECISION NOT NULL,
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
    last_heartbeat  DOUBLE PRECISION NOT NULL,
    registered_at   DOUBLE PRECISION NOT NULL
);

CREATE TABLE IF NOT EXISTS workflow_snapshots (
    workflow_id     TEXT NOT NULL REFERENCES workflows(id),
    event_seq       INTEGER NOT NULL,
    state_json      TEXT NOT NULL,
    created_at      DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (workflow_id, event_seq)
);

CREATE TABLE IF NOT EXISTS api_keys (
    key_hash        TEXT PRIMARY KEY,
    prefix          TEXT NOT NULL,
    label           TEXT,
    created_at      DOUBLE PRECISION NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_api_keys_prefix ON api_keys(prefix);

-- Future additive migrations go below this line. Postgres supports
-- `ADD COLUMN IF NOT EXISTS` natively, so the pattern is simply:
--
--   ALTER TABLE workflows ADD COLUMN IF NOT EXISTS some_new_field TEXT;
--
-- Idempotent across startups; fresh installs pick the column up from the
-- CREATE TABLE above so the ADD is a no-op. Currently no pending
-- migrations — baseline schema in CREATE TABLE statements above is the
-- source of truth through v0.11.3.
"#;

/// Split a Postgres DDL script into individual statements ready for `sqlx::query`.
///
/// Drops pure-comment lines (those starting with `--` after optional whitespace)
/// *before* splitting on `;`. Without this step, a semicolon inside a line comment
/// (e.g. `-- Idempotent across startups; fresh installs pick the column up`) would
/// split the surrounding comment into fragments — one of which is naked prose that
/// Postgres tries to parse as SQL and rejects with `syntax error at or near "<word>"`.
///
/// The filter only drops *pure-comment* lines (leading whitespace then `--`), leaving
/// `--`-after-code untouched. That keeps string literals safe (could legally contain
/// `--`) and is conservative enough to remain correct if the SCHEMA grows more prose.
fn sanitise_schema(schema: &str) -> Vec<String> {
    let without_comments: String = schema
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");

    without_comments
        .split(';')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub struct PostgresStore {
    pool: PgPool,
}

impl PostgresStore {
    pub async fn new(url: &str) -> Result<Self> {
        let pool = PgPool::connect(url).await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> Result<()> {
        for statement in sanitise_schema(SCHEMA) {
            sqlx::query(&statement).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Try to acquire pg_advisory_lock for leader election.
    /// Returns true if this instance is the leader (scheduler should run).
    pub async fn try_acquire_leader_lock(&self) -> Result<bool> {
        let row: (bool,) =
            sqlx::query_as("SELECT pg_try_advisory_lock(1)")
                .fetch_one(&self.pool)
                .await?;
        Ok(row.0)
    }
}

impl WorkflowStore for PostgresStore {
    // ── Namespaces ─────────────────────────────────────────

    async fn create_namespace(&self, name: &str) -> Result<()> {
        sqlx::query("INSERT INTO namespaces (name, created_at) VALUES ($1, EXTRACT(EPOCH FROM NOW()))")
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_namespaces(&self) -> Result<Vec<crate::store::NamespaceRecord>> {
        let rows = sqlx::query_as::<_, (String, f64)>(
            "SELECT name, created_at FROM namespaces ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(name, created_at)| crate::store::NamespaceRecord { name, created_at })
            .collect())
    }

    async fn delete_namespace(&self, name: &str) -> Result<bool> {
        let res = sqlx::query("DELETE FROM namespaces WHERE name = $1 AND name != 'main'")
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn get_namespace_stats(&self, namespace: &str) -> Result<crate::store::NamespaceStats> {
        let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM workflows WHERE namespace = $1")
            .bind(namespace)
            .fetch_one(&self.pool)
            .await?;
        let running: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM workflows WHERE namespace = $1 AND status = 'RUNNING'",
        )
        .bind(namespace)
        .fetch_one(&self.pool)
        .await?;
        let pending: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM workflows WHERE namespace = $1 AND status = 'PENDING'",
        )
        .bind(namespace)
        .fetch_one(&self.pool)
        .await?;
        let completed: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM workflows WHERE namespace = $1 AND status = 'COMPLETED'",
        )
        .bind(namespace)
        .fetch_one(&self.pool)
        .await?;
        let failed: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM workflows WHERE namespace = $1 AND status = 'FAILED'",
        )
        .bind(namespace)
        .fetch_one(&self.pool)
        .await?;
        let schedules: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM workflow_schedules WHERE namespace = $1")
                .bind(namespace)
                .fetch_one(&self.pool)
                .await?;
        let workers: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM workflow_workers WHERE namespace = $1")
                .bind(namespace)
                .fetch_one(&self.pool)
                .await?;

        Ok(crate::store::NamespaceStats {
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
            "INSERT INTO workflows (id, namespace, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, search_attributes, archived_at, archive_uri, created_at, updated_at, completed_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
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
        .bind(&wf.search_attributes)
        .bind(wf.archived_at)
        .bind(&wf.archive_uri)
        .bind(wf.created_at)
        .bind(wf.updated_at)
        .bind(wf.completed_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_workflow(&self, id: &str) -> Result<Option<WorkflowRecord>> {
        let row = sqlx::query_as::<_, PgWorkflowRow>(
            "SELECT id, namespace, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, search_attributes, archived_at, archive_uri, created_at, updated_at, completed_at FROM workflows WHERE id = $1",
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
        search_attrs_filter: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<WorkflowRecord>> {
        let status_str = status.map(|s| s.to_string());

        let filter_pairs: Vec<(String, serde_json::Value)> = search_attrs_filter
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v.as_object().cloned())
            .map(|m| m.into_iter().collect())
            .unwrap_or_default();

        let mut sql = String::from(
            "SELECT id, namespace, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, search_attributes, archived_at, archive_uri, created_at, updated_at, completed_at
             FROM workflows
             WHERE namespace = $1
               AND ($2::TEXT IS NULL OR status = $2)
               AND ($3::TEXT IS NULL OR workflow_type = $3)",
        );
        // Bind placeholders for the filter follow $3; next index is 4.
        let mut idx = 4usize;
        for _ in &filter_pairs {
            sql.push_str(&format!(
                " AND (search_attributes::jsonb)->>${} = ${}",
                idx,
                idx + 1
            ));
            idx += 2;
        }
        sql.push_str(&format!(" ORDER BY created_at DESC LIMIT ${} OFFSET ${}", idx, idx + 1));

        let mut q = sqlx::query_as::<_, PgWorkflowRow>(&sql)
            .bind(namespace)
            .bind(&status_str)
            .bind(workflow_type);
        for (key, value) in &filter_pairs {
            q = q.bind(key.clone());
            // JSONB ->> always returns TEXT; compare by stringified value.
            let as_text = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            q = q.bind(as_text);
        }
        let rows = q.bind(limit).bind(offset).fetch_all(&self.pool).await?;
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
            "UPDATE workflows SET status = $1, result = COALESCE($2, result), error = COALESCE($3, error), updated_at = $4, completed_at = COALESCE($5, completed_at) WHERE id = $6",
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
            "UPDATE workflows SET claimed_by = $1, status = 'RUNNING', updated_at = $2 WHERE id = $3 AND claimed_by IS NULL",
        )
        .bind(worker_id)
        .bind(timestamp_now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn mark_workflow_dispatchable(&self, workflow_id: &str) -> Result<()> {
        sqlx::query("UPDATE workflows SET needs_dispatch = TRUE WHERE id = $1")
            .bind(workflow_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn claim_workflow_task(
        &self,
        task_queue: &str,
        worker_id: &str,
    ) -> Result<Option<WorkflowRecord>> {
        let now = timestamp_now();
        // Atomic claim with FOR UPDATE SKIP LOCKED so multiple engine
        // replicas don't fight over the same workflow task.
        let row = sqlx::query_as::<_, PgWorkflowRow>(
            "UPDATE workflows
             SET dispatch_claimed_by = $1, dispatch_last_heartbeat = $2, needs_dispatch = FALSE
             WHERE id = (
                SELECT id FROM workflows
                WHERE task_queue = $3
                  AND needs_dispatch = TRUE
                  AND dispatch_claimed_by IS NULL
                  AND status NOT IN ('COMPLETED', 'FAILED', 'CANCELLED', 'TIMED_OUT')
                ORDER BY updated_at ASC
                FOR UPDATE SKIP LOCKED
                LIMIT 1
             )
             RETURNING id, namespace, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, search_attributes, archived_at, archive_uri, created_at, updated_at, completed_at",
        )
        .bind(worker_id)
        .bind(now)
        .bind(task_queue)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    async fn release_workflow_task(&self, workflow_id: &str, worker_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE workflows
             SET dispatch_claimed_by = NULL, dispatch_last_heartbeat = NULL
             WHERE id = $1 AND dispatch_claimed_by = $2",
        )
        .bind(workflow_id)
        .bind(worker_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn release_stale_dispatch_leases(
        &self,
        now: f64,
        timeout_secs: f64,
    ) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE workflows
             SET dispatch_claimed_by = NULL,
                 dispatch_last_heartbeat = NULL,
                 needs_dispatch = TRUE
             WHERE dispatch_claimed_by IS NOT NULL
               AND ($1 - dispatch_last_heartbeat) > $2
               AND status NOT IN ('COMPLETED', 'FAILED', 'CANCELLED', 'TIMED_OUT')",
        )
        .bind(now)
        .bind(timeout_secs)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    // ── Events ─────────────────────────────────────────────

    async fn append_event(&self, ev: &WorkflowEvent) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "INSERT INTO workflow_events (workflow_id, seq, event_type, payload, timestamp) VALUES ($1, $2, $3, $4, $5) RETURNING id",
        )
        .bind(&ev.workflow_id)
        .bind(ev.seq)
        .bind(&ev.event_type)
        .bind(&ev.payload)
        .bind(ev.timestamp)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    async fn list_events(&self, workflow_id: &str) -> Result<Vec<WorkflowEvent>> {
        let rows = sqlx::query_as::<_, PgEventRow>(
            "SELECT id, workflow_id, seq, event_type, payload, timestamp FROM workflow_events WHERE workflow_id = $1 ORDER BY seq ASC",
        )
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get_event_count(&self, workflow_id: &str) -> Result<i64> {
        let row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM workflow_events WHERE workflow_id = $1")
                .bind(workflow_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(row.0)
    }

    // ── Activities ──────────────────────────────────────────

    async fn create_activity(&self, act: &WorkflowActivity) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "INSERT INTO workflow_activities (workflow_id, seq, name, task_queue, input, status, attempt, max_attempts, initial_interval_secs, backoff_coefficient, start_to_close_secs, heartbeat_timeout_secs, scheduled_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) RETURNING id",
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
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    async fn get_activity(&self, id: i64) -> Result<Option<WorkflowActivity>> {
        let row = sqlx::query_as::<_, PgActivityRow>(
            "SELECT id, workflow_id, seq, name, task_queue, input, status, result, error, attempt, max_attempts, initial_interval_secs, backoff_coefficient, start_to_close_secs, heartbeat_timeout_secs, claimed_by, scheduled_at, started_at, completed_at, last_heartbeat
             FROM workflow_activities WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    async fn get_activity_by_workflow_seq(
        &self,
        workflow_id: &str,
        seq: i32,
    ) -> Result<Option<WorkflowActivity>> {
        let row = sqlx::query_as::<_, PgActivityRow>(
            "SELECT id, workflow_id, seq, name, task_queue, input, status, result, error, attempt, max_attempts, initial_interval_secs, backoff_coefficient, start_to_close_secs, heartbeat_timeout_secs, claimed_by, scheduled_at, started_at, completed_at, last_heartbeat
             FROM workflow_activities WHERE workflow_id = $1 AND seq = $2",
        )
        .bind(workflow_id)
        .bind(seq)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    async fn claim_activity(
        &self,
        task_queue: &str,
        worker_id: &str,
    ) -> Result<Option<WorkflowActivity>> {
        let now = timestamp_now();
        // Atomic claim using FOR UPDATE SKIP LOCKED — prevents contention
        // between multiple assay serve instances claiming the same activity
        let row = sqlx::query_as::<_, PgActivityRow>(
            "UPDATE workflow_activities SET status = 'RUNNING', claimed_by = $1, started_at = $2
             WHERE id = (
                SELECT id FROM workflow_activities
                WHERE task_queue = $3 AND status = 'PENDING'
                ORDER BY scheduled_at ASC
                FOR UPDATE SKIP LOCKED
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

    async fn requeue_activity_for_retry(
        &self,
        id: i64,
        next_attempt: i32,
        next_scheduled_at: f64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE workflow_activities
             SET status = 'PENDING', attempt = $1, scheduled_at = $2,
                 claimed_by = NULL, started_at = NULL, last_heartbeat = NULL,
                 error = NULL
             WHERE id = $3",
        )
        .bind(next_attempt)
        .bind(next_scheduled_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
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
            "UPDATE workflow_activities SET status = $1, result = $2, error = $3, completed_at = $4 WHERE id = $5",
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
        sqlx::query("UPDATE workflow_activities SET last_heartbeat = $1 WHERE id = $2")
            .bind(timestamp_now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_timed_out_activities(&self, now: f64) -> Result<Vec<WorkflowActivity>> {
        let rows = sqlx::query_as::<_, PgActivityRow>(
            "SELECT id, workflow_id, seq, name, task_queue, input, status, result, error, attempt, max_attempts, initial_interval_secs, backoff_coefficient, start_to_close_secs, heartbeat_timeout_secs, claimed_by, scheduled_at, started_at, completed_at, last_heartbeat
             FROM workflow_activities
             WHERE status = 'RUNNING'
               AND heartbeat_timeout_secs IS NOT NULL
               AND last_heartbeat IS NOT NULL
               AND ($1 - last_heartbeat) > heartbeat_timeout_secs",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    // ── Timers ──────────────────────────────────────────────

    async fn create_timer(&self, timer: &WorkflowTimer) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "INSERT INTO workflow_timers (workflow_id, seq, fire_at, fired) VALUES ($1, $2, $3, FALSE) RETURNING id",
        )
        .bind(&timer.workflow_id)
        .bind(timer.seq)
        .bind(timer.fire_at)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    async fn cancel_pending_activities(&self, workflow_id: &str) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE workflow_activities SET status = 'CANCELLED', completed_at = $1
             WHERE workflow_id = $2 AND status = 'PENDING'",
        )
        .bind(timestamp_now())
        .bind(workflow_id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    async fn cancel_pending_timers(&self, workflow_id: &str) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE workflow_timers SET fired = TRUE
             WHERE workflow_id = $1 AND fired = FALSE",
        )
        .bind(workflow_id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    async fn get_timer_by_workflow_seq(
        &self,
        workflow_id: &str,
        seq: i32,
    ) -> Result<Option<WorkflowTimer>> {
        let row = sqlx::query_as::<_, PgTimerRow>(
            "SELECT id, workflow_id, seq, fire_at, fired
             FROM workflow_timers WHERE workflow_id = $1 AND seq = $2",
        )
        .bind(workflow_id)
        .bind(seq)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    async fn fire_due_timers(&self, now: f64) -> Result<Vec<WorkflowTimer>> {
        let rows = sqlx::query_as::<_, PgTimerRow>(
            "UPDATE workflow_timers SET fired = TRUE
             WHERE fired = FALSE AND fire_at <= $1
             RETURNING id, workflow_id, seq, fire_at, fired",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    // ── Signals ─────────────────────────────────────────────

    async fn send_signal(&self, sig: &WorkflowSignal) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "INSERT INTO workflow_signals (workflow_id, name, payload, consumed, received_at) VALUES ($1, $2, $3, FALSE, $4) RETURNING id",
        )
        .bind(&sig.workflow_id)
        .bind(&sig.name)
        .bind(&sig.payload)
        .bind(sig.received_at)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    async fn consume_signals(
        &self,
        workflow_id: &str,
        name: &str,
    ) -> Result<Vec<WorkflowSignal>> {
        let rows = sqlx::query_as::<_, PgSignalRow>(
            "UPDATE workflow_signals SET consumed = TRUE
             WHERE workflow_id = $1 AND name = $2 AND consumed = FALSE
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
            "INSERT INTO workflow_schedules (namespace, name, workflow_type, cron_expr, timezone, input, task_queue, overlap_policy, paused, last_run_at, next_run_at, last_workflow_id, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        )
        .bind(&sched.namespace)
        .bind(&sched.name)
        .bind(&sched.workflow_type)
        .bind(&sched.cron_expr)
        .bind(&sched.timezone)
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
        let row = sqlx::query_as::<_, PgScheduleRow>(
            "SELECT namespace, name, workflow_type, cron_expr, timezone, input, task_queue, overlap_policy, paused, last_run_at, next_run_at, last_workflow_id, created_at FROM workflow_schedules WHERE namespace = $1 AND name = $2",
        )
        .bind(namespace)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    async fn list_schedules(&self, namespace: &str) -> Result<Vec<WorkflowSchedule>> {
        let rows = sqlx::query_as::<_, PgScheduleRow>(
            "SELECT namespace, name, workflow_type, cron_expr, timezone, input, task_queue, overlap_policy, paused, last_run_at, next_run_at, last_workflow_id, created_at FROM workflow_schedules WHERE namespace = $1 ORDER BY name",
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
            "UPDATE workflow_schedules SET last_run_at = $1, next_run_at = $2, last_workflow_id = $3 WHERE namespace = $4 AND name = $5",
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
        let res = sqlx::query("DELETE FROM workflow_schedules WHERE namespace = $1 AND name = $2")
            .bind(namespace)
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn list_archivable_workflows(
        &self,
        cutoff: f64,
        limit: i64,
    ) -> Result<Vec<WorkflowRecord>> {
        let rows = sqlx::query_as::<_, PgWorkflowRow>(
            "SELECT id, namespace, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, search_attributes, archived_at, archive_uri, created_at, updated_at, completed_at
             FROM workflows
             WHERE status IN ('COMPLETED', 'FAILED', 'CANCELLED', 'TIMED_OUT')
               AND completed_at IS NOT NULL
               AND completed_at < $1
               AND archived_at IS NULL
             ORDER BY completed_at ASC
             LIMIT $2",
        )
        .bind(cutoff)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn mark_archived_and_purge(
        &self,
        workflow_id: &str,
        archive_uri: &str,
        archived_at: f64,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM workflow_events WHERE workflow_id = $1")
            .bind(workflow_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM workflow_activities WHERE workflow_id = $1")
            .bind(workflow_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM workflow_timers WHERE workflow_id = $1")
            .bind(workflow_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM workflow_signals WHERE workflow_id = $1")
            .bind(workflow_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM workflow_snapshots WHERE workflow_id = $1")
            .bind(workflow_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE workflows SET archived_at = $1, archive_uri = $2 WHERE id = $3",
        )
        .bind(archived_at)
        .bind(archive_uri)
        .bind(workflow_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn upsert_search_attributes(
        &self,
        workflow_id: &str,
        patch_json: &str,
    ) -> Result<()> {
        let current: Option<(Option<String>,)> =
            sqlx::query_as("SELECT search_attributes FROM workflows WHERE id = $1")
                .bind(workflow_id)
                .fetch_optional(&self.pool)
                .await?;
        let merged = crate::store::sqlite::merge_search_attrs(
            current.and_then(|(s,)| s).as_deref(),
            patch_json,
        )?;
        sqlx::query("UPDATE workflows SET search_attributes = $1 WHERE id = $2")
            .bind(merged)
            .bind(workflow_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn update_schedule(
        &self,
        namespace: &str,
        name: &str,
        patch: &SchedulePatch,
    ) -> Result<Option<WorkflowSchedule>> {
        let mut sets: Vec<String> = Vec::new();
        let mut idx = 1usize;
        if patch.cron_expr.is_some() {
            sets.push(format!("cron_expr = ${idx}"));
            idx += 1;
        }
        if patch.timezone.is_some() {
            sets.push(format!("timezone = ${idx}"));
            idx += 1;
        }
        if patch.input.is_some() {
            sets.push(format!("input = ${idx}"));
            idx += 1;
        }
        if patch.task_queue.is_some() {
            sets.push(format!("task_queue = ${idx}"));
            idx += 1;
        }
        if patch.overlap_policy.is_some() {
            sets.push(format!("overlap_policy = ${idx}"));
            idx += 1;
        }
        if sets.is_empty() {
            return self.get_schedule(namespace, name).await;
        }
        let sql = format!(
            "UPDATE workflow_schedules SET {} WHERE namespace = ${} AND name = ${}",
            sets.join(", "),
            idx,
            idx + 1
        );
        let mut q = sqlx::query(&sql);
        if let Some(ref v) = patch.cron_expr {
            q = q.bind(v);
        }
        if let Some(ref v) = patch.timezone {
            q = q.bind(v);
        }
        if let Some(ref v) = patch.input {
            q = q.bind(v.to_string());
        }
        if let Some(ref v) = patch.task_queue {
            q = q.bind(v);
        }
        if let Some(ref v) = patch.overlap_policy {
            q = q.bind(v);
        }
        let res = q
            .bind(namespace)
            .bind(name)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_schedule(namespace, name).await
    }

    async fn set_schedule_paused(
        &self,
        namespace: &str,
        name: &str,
        paused: bool,
    ) -> Result<Option<WorkflowSchedule>> {
        let res = sqlx::query(
            "UPDATE workflow_schedules SET paused = $1 WHERE namespace = $2 AND name = $3",
        )
        .bind(paused)
        .bind(namespace)
        .bind(name)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_schedule(namespace, name).await
    }

    // ── Workers ─────────────────────────────────────────────

    async fn register_worker(&self, w: &WorkflowWorker) -> Result<()> {
        sqlx::query(
            "INSERT INTO workflow_workers (id, namespace, identity, task_queue, workflows, activities, max_concurrent_workflows, max_concurrent_activities, active_tasks, last_heartbeat, registered_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             ON CONFLICT (id) DO UPDATE SET last_heartbeat = EXCLUDED.last_heartbeat, identity = EXCLUDED.identity",
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
        sqlx::query("UPDATE workflow_workers SET last_heartbeat = $1 WHERE id = $2")
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_workers(&self, namespace: &str) -> Result<Vec<WorkflowWorker>> {
        let rows = sqlx::query_as::<_, PgWorkerRow>(
            "SELECT id, namespace, identity, task_queue, workflows, activities, max_concurrent_workflows, max_concurrent_activities, active_tasks, last_heartbeat, registered_at FROM workflow_workers WHERE namespace = $1 ORDER BY registered_at",
        )
        .bind(namespace)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn remove_dead_workers(&self, cutoff: f64) -> Result<Vec<String>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT id FROM workflow_workers WHERE last_heartbeat < $1")
                .bind(cutoff)
                .fetch_all(&self.pool)
                .await?;
        let ids: Vec<String> = rows.into_iter().map(|r| r.0).collect();
        if !ids.is_empty() {
            sqlx::query("DELETE FROM workflow_workers WHERE last_heartbeat < $1")
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
        sqlx::query("INSERT INTO api_keys (key_hash, prefix, label, created_at) VALUES ($1, $2, $3, $4)")
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
            sqlx::query_as("SELECT 1::BIGINT FROM api_keys WHERE key_hash = $1")
                .bind(key_hash)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.is_some())
    }

    async fn list_api_keys(&self) -> Result<Vec<crate::store::ApiKeyRecord>> {
        let rows = sqlx::query_as::<_, (String, Option<String>, f64)>(
            "SELECT prefix, label, created_at FROM api_keys ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(prefix, label, created_at)| crate::store::ApiKeyRecord {
                prefix,
                label,
                created_at,
            })
            .collect())
    }

    async fn revoke_api_key(&self, prefix: &str) -> Result<bool> {
        let res = sqlx::query("DELETE FROM api_keys WHERE prefix = $1")
            .bind(prefix)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    // ── Child Workflows ─────────────────────────────────────

    async fn list_child_workflows(&self, parent_id: &str) -> Result<Vec<WorkflowRecord>> {
        let rows = sqlx::query_as::<_, PgWorkflowRow>(
            "SELECT id, namespace, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, search_attributes, archived_at, archive_uri, created_at, updated_at, completed_at
             FROM workflows WHERE parent_id = $1 ORDER BY created_at ASC",
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
            "INSERT INTO workflow_snapshots (workflow_id, event_seq, state_json, created_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (workflow_id, event_seq) DO UPDATE SET state_json = EXCLUDED.state_json, created_at = EXCLUDED.created_at",
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
             FROM workflow_snapshots WHERE workflow_id = $1
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

    async fn get_queue_stats(&self, namespace: &str) -> Result<Vec<crate::store::QueueStats>> {
        let rows = sqlx::query_as::<_, (String, i64, i64, i64)>(
            "SELECT
                a.task_queue AS queue,
                SUM(CASE WHEN a.status = 'PENDING' THEN 1 ELSE 0 END) AS pending,
                SUM(CASE WHEN a.status = 'RUNNING' THEN 1 ELSE 0 END) AS running,
                (SELECT COUNT(*) FROM workflow_workers w WHERE w.task_queue = a.task_queue AND w.namespace = $1) AS workers
             FROM workflow_activities a
             JOIN workflows wf ON a.workflow_id = wf.id AND wf.namespace = $1
             GROUP BY a.task_queue",
        )
        .bind(namespace)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(queue, pending, running, workers)| crate::store::QueueStats {
                queue,
                pending_activities: pending,
                running_activities: running,
                workers,
            })
            .collect())
    }

    // ── Leader Election ─────────────────────────────────────

    async fn try_acquire_scheduler_lock(&self) -> Result<bool> {
        // pg_try_advisory_lock is session-scoped — only one connection
        // in the pool will hold the lock. In a multi-replica Kubernetes
        // deployment, only one pod's connection wins.
        let row: (bool,) =
            sqlx::query_as("SELECT pg_try_advisory_lock(42)")
                .fetch_one(&self.pool)
                .await?;
        Ok(row.0)
    }
}

fn timestamp_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

// ── Postgres row types (sqlx::FromRow) ──────────────────────

#[derive(sqlx::FromRow)]
struct PgWorkflowRow {
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
    search_attributes: Option<String>,
    archived_at: Option<f64>,
    archive_uri: Option<String>,
    created_at: f64,
    updated_at: f64,
    completed_at: Option<f64>,
}

impl From<PgWorkflowRow> for WorkflowRecord {
    fn from(r: PgWorkflowRow) -> Self {
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
            search_attributes: r.search_attributes,
            archived_at: r.archived_at,
            archive_uri: r.archive_uri,
            created_at: r.created_at,
            updated_at: r.updated_at,
            completed_at: r.completed_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct PgEventRow {
    id: i64,
    workflow_id: String,
    seq: i32,
    event_type: String,
    payload: Option<String>,
    timestamp: f64,
}

impl From<PgEventRow> for WorkflowEvent {
    fn from(r: PgEventRow) -> Self {
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
struct PgActivityRow {
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

impl From<PgActivityRow> for WorkflowActivity {
    fn from(r: PgActivityRow) -> Self {
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
struct PgTimerRow {
    id: i64,
    workflow_id: String,
    seq: i32,
    fire_at: f64,
    fired: bool,
}

impl From<PgTimerRow> for WorkflowTimer {
    fn from(r: PgTimerRow) -> Self {
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
struct PgSignalRow {
    id: i64,
    workflow_id: String,
    name: String,
    payload: Option<String>,
    consumed: bool,
    received_at: f64,
}

impl From<PgSignalRow> for WorkflowSignal {
    fn from(r: PgSignalRow) -> Self {
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
struct PgScheduleRow {
    namespace: String,
    name: String,
    workflow_type: String,
    cron_expr: String,
    timezone: String,
    input: Option<String>,
    task_queue: String,
    overlap_policy: String,
    paused: bool,
    last_run_at: Option<f64>,
    next_run_at: Option<f64>,
    last_workflow_id: Option<String>,
    created_at: f64,
}

impl From<PgScheduleRow> for WorkflowSchedule {
    fn from(r: PgScheduleRow) -> Self {
        Self {
            namespace: r.namespace,
            name: r.name,
            workflow_type: r.workflow_type,
            cron_expr: r.cron_expr,
            timezone: r.timezone,
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
struct PgWorkerRow {
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

impl From<PgWorkerRow> for WorkflowWorker {
    fn from(r: PgWorkerRow) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitise_schema_keeps_statements_intact() {
        let input = "CREATE TABLE foo (x INT);\nCREATE INDEX idx_foo ON foo(x);\n";
        let out = sanitise_schema(input);
        assert_eq!(out.len(), 2);
        assert!(out[0].starts_with("CREATE TABLE foo"));
        assert!(out[1].starts_with("CREATE INDEX idx_foo"));
    }

    #[test]
    fn sanitise_schema_drops_pure_comment_lines() {
        let input = "-- header comment\nCREATE TABLE foo (x INT);\n-- trailing comment\n";
        let out = sanitise_schema(input);
        assert_eq!(out.len(), 1);
        assert!(out[0].starts_with("CREATE TABLE foo"));
    }

    #[test]
    fn sanitise_schema_ignores_semicolons_inside_comment_prose() {
        // Regression: the exact shape that broke v0.11.3–v0.11.5 in production.
        // `-- foo; bar` used to split into "foo" and " bar" fragments, the second
        // of which was executed as SQL and rejected with `syntax error at or near "bar"`.
        let input = "\
CREATE TABLE foo (x INT);
-- Idempotent across startups; fresh installs pick the column up from the
-- CREATE TABLE above so the ADD is a no-op.
";
        let out = sanitise_schema(input);
        assert_eq!(
            out.len(),
            1,
            "expected 1 real statement, got {}: {:?}",
            out.len(),
            out
        );
        assert!(out[0].starts_with("CREATE TABLE foo"));
    }

    #[test]
    fn sanitise_schema_drops_indented_comment_lines() {
        let input = "  -- indented comment\n\tCREATE TABLE foo (x INT);\n";
        let out = sanitise_schema(input);
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("CREATE TABLE foo"));
    }

    #[test]
    fn sanitise_schema_real_constant_produces_only_ddl() {
        // The real SCHEMA constant must not produce any statement whose first
        // token isn't a recognised SQL keyword. A prose fragment leaking in
        // (e.g. "fresh installs...") means the filter regressed.
        for stmt in sanitise_schema(SCHEMA) {
            let first_word = stmt
                .split_whitespace()
                .next()
                .expect("non-empty statement")
                .to_uppercase();
            assert!(
                matches!(
                    first_word.as_str(),
                    "CREATE" | "INSERT" | "UPDATE" | "DROP" | "ALTER" | "WITH"
                ),
                "SCHEMA produced non-DDL statement starting with {first_word:?}: {stmt:?}"
            );
        }
    }
}
