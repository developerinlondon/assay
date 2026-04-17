pub mod postgres;
pub mod sqlite;

use std::future::Future;

use crate::types::*;

/// Core storage trait for the workflow engine.
///
/// All database access goes through this trait. Methods that operate on
/// namespace-scoped data take a `namespace` parameter. The "main"
/// namespace is always available.
///
/// All methods return `Send` futures so they can be used from `tokio::spawn`.
pub trait WorkflowStore: Send + Sync + 'static {
    // ── Namespaces ─────────────────────────────────────────

    fn create_namespace(
        &self,
        name: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn list_namespaces(
        &self,
    ) -> impl Future<Output = anyhow::Result<Vec<NamespaceRecord>>> + Send;

    fn delete_namespace(
        &self,
        name: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;

    fn get_namespace_stats(
        &self,
        namespace: &str,
    ) -> impl Future<Output = anyhow::Result<NamespaceStats>> + Send;

    // ── Workflows ──────────────────────────────────────────

    fn create_workflow(
        &self,
        workflow: &WorkflowRecord,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn get_workflow(
        &self,
        id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowRecord>>> + Send;

    fn list_workflows(
        &self,
        namespace: &str,
        status: Option<WorkflowStatus>,
        workflow_type: Option<&str>,
        search_attrs_filter: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowRecord>>> + Send;

    /// Merge a JSON object patch into the workflow's `search_attributes`.
    /// Keys in the patch overwrite existing keys; keys already present but
    /// not in the patch are preserved. If the current column is NULL, the
    /// patch becomes the new value.
    fn upsert_search_attributes(
        &self,
        workflow_id: &str,
        patch_json: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn update_workflow_status(
        &self,
        id: &str,
        status: WorkflowStatus,
        result: Option<&str>,
        error: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn claim_workflow(
        &self,
        id: &str,
        worker_id: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;

    // ── Workflow-task dispatch (Phase 9) ────────────────────

    /// Mark a workflow as having new events that need a worker to replay it.
    /// Idempotent — calling repeatedly is fine. Cleared by `claim_workflow_task`.
    fn mark_workflow_dispatchable(
        &self,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Atomically claim the oldest dispatchable workflow on a queue. Sets
    /// `dispatch_claimed_by` and `dispatch_last_heartbeat`, clears
    /// `needs_dispatch`. Returns the workflow record or None if nothing
    /// is available.
    fn claim_workflow_task(
        &self,
        task_queue: &str,
        worker_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowRecord>>> + Send;

    /// Release a workflow task's dispatch lease (called when the worker
    /// submits its commands batch). Only succeeds if `dispatch_claimed_by`
    /// matches the calling worker.
    fn release_workflow_task(
        &self,
        workflow_id: &str,
        worker_id: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Forcibly release dispatch leases whose worker hasn't heartbeat'd
    /// within `timeout_secs`. Used by the engine's background poller to
    /// recover from worker crashes. Returns how many leases were released
    /// (each becomes claimable again, with `needs_dispatch=true`).
    fn release_stale_dispatch_leases(
        &self,
        now: f64,
        timeout_secs: f64,
    ) -> impl Future<Output = anyhow::Result<u64>> + Send;

    // ── Events ─────────────────────────────────────────────

    fn append_event(
        &self,
        event: &WorkflowEvent,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send;

    fn list_events(
        &self,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowEvent>>> + Send;

    fn get_event_count(
        &self,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send;

    // ── Activities ──────────────────────────────────────────

    fn create_activity(
        &self,
        activity: &WorkflowActivity,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send;

    /// Look up an activity by its primary key.
    fn get_activity(
        &self,
        id: i64,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowActivity>>> + Send;

    /// Look up an activity by its workflow-relative sequence number.
    /// Used for idempotent scheduling: the engine checks if (workflow_id, seq)
    /// already exists before creating a new row.
    fn get_activity_by_workflow_seq(
        &self,
        workflow_id: &str,
        seq: i32,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowActivity>>> + Send;

    fn claim_activity(
        &self,
        task_queue: &str,
        worker_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowActivity>>> + Send;

    /// Re-queue an activity for retry: clears the running state
    /// (status→PENDING, claimed_by/started_at cleared), bumps `attempt`,
    /// and sets `scheduled_at = now + backoff` so the next claim_activity
    /// won't pick it up before the backoff elapses.
    fn requeue_activity_for_retry(
        &self,
        id: i64,
        next_attempt: i32,
        next_scheduled_at: f64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn complete_activity(
        &self,
        id: i64,
        result: Option<&str>,
        error: Option<&str>,
        failed: bool,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn heartbeat_activity(
        &self,
        id: i64,
        details: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn get_timed_out_activities(
        &self,
        now: f64,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowActivity>>> + Send;

    // ── Timers ──────────────────────────────────────────────

    /// Mark all PENDING activities of a workflow as CANCELLED so workers
    /// that haven't claimed them yet won't pick them up. Returns the
    /// number of rows affected. Does NOT touch RUNNING activities — those
    /// will see the cancellation when they next heartbeat or complete.
    fn cancel_pending_activities(
        &self,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<u64>> + Send;

    /// Mark all unfired timers of a workflow as fired without firing
    /// (effectively removing them from the timer poller). Returns the
    /// number of rows affected.
    fn cancel_pending_timers(
        &self,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<u64>> + Send;

    fn create_timer(
        &self,
        timer: &WorkflowTimer,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send;

    /// Look up an existing timer by its workflow-relative seq. Used by the
    /// engine for idempotent ScheduleTimer (deterministic replay can call
    /// schedule_timer for the same seq more than once on retries).
    fn get_timer_by_workflow_seq(
        &self,
        workflow_id: &str,
        seq: i32,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowTimer>>> + Send;

    fn fire_due_timers(
        &self,
        now: f64,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowTimer>>> + Send;

    // ── Signals ─────────────────────────────────────────────

    fn send_signal(
        &self,
        signal: &WorkflowSignal,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send;

    fn consume_signals(
        &self,
        workflow_id: &str,
        name: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowSignal>>> + Send;

    // ── Schedules ───────────────────────────────────────────

    fn create_schedule(
        &self,
        schedule: &WorkflowSchedule,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn get_schedule(
        &self,
        namespace: &str,
        name: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowSchedule>>> + Send;

    fn list_schedules(
        &self,
        namespace: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowSchedule>>> + Send;

    fn update_schedule_last_run(
        &self,
        namespace: &str,
        name: &str,
        last_run_at: f64,
        next_run_at: f64,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn delete_schedule(
        &self,
        namespace: &str,
        name: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;

    /// Apply an in-place patch to a schedule. Only fields present on
    /// `patch` are updated; the rest keep their current values. Returns
    /// the updated record, or `None` if the schedule doesn't exist.
    ///
    /// The scheduler's `next_run_at` is recomputed from the new
    /// `cron_expr` + `timezone` on the next evaluation tick, so a PATCH
    /// takes effect within the scheduler's poll interval.
    fn update_schedule(
        &self,
        namespace: &str,
        name: &str,
        patch: &SchedulePatch,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowSchedule>>> + Send;

    /// Flip a schedule's `paused` flag. Returns the updated record, or
    /// `None` if the schedule doesn't exist.
    ///
    /// A paused schedule is skipped by the scheduler; resuming it
    /// doesn't backfill missed fires — the next fire is whatever the
    /// cron expression says, starting from now.
    fn set_schedule_paused(
        &self,
        namespace: &str,
        name: &str,
        paused: bool,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowSchedule>>> + Send;

    // ── Workers ─────────────────────────────────────────────

    fn register_worker(
        &self,
        worker: &WorkflowWorker,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn heartbeat_worker(
        &self,
        id: &str,
        now: f64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn list_workers(
        &self,
        namespace: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowWorker>>> + Send;

    fn remove_dead_workers(
        &self,
        cutoff: f64,
    ) -> impl Future<Output = anyhow::Result<Vec<String>>> + Send;

    // ── API Keys ────────────────────────────────────────────

    fn create_api_key(
        &self,
        key_hash: &str,
        prefix: &str,
        label: Option<&str>,
        created_at: f64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn validate_api_key(
        &self,
        key_hash: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;

    fn list_api_keys(
        &self,
    ) -> impl Future<Output = anyhow::Result<Vec<ApiKeyRecord>>> + Send;

    fn revoke_api_key(
        &self,
        prefix: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;

    // ── Child Workflows ─────────────────────────────────────

    fn list_child_workflows(
        &self,
        parent_id: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowRecord>>> + Send;

    // ── Snapshots ───────────────────────────────────────────

    fn create_snapshot(
        &self,
        workflow_id: &str,
        event_seq: i32,
        state_json: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn get_latest_snapshot(
        &self,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowSnapshot>>> + Send;

    // ── Queue Stats ─────────────────────────────────────────

    fn get_queue_stats(
        &self,
        namespace: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<QueueStats>>> + Send;

    // ── Leader Election ─────────────────────────────────────

    /// Try to acquire the scheduler lock for leader election.
    /// Returns true if this instance should run the cron scheduler.
    ///
    /// - SQLite: always returns true (single-instance assumed)
    /// - Postgres: uses pg_try_advisory_lock (only one instance wins)
    fn try_acquire_scheduler_lock(
        &self,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;
}

/// API key metadata (hash is never exposed).
#[derive(Clone, Debug, serde::Serialize)]
pub struct ApiKeyRecord {
    pub prefix: String,
    pub label: Option<String>,
    pub created_at: f64,
}

/// Namespace record.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct NamespaceRecord {
    pub name: String,
    pub created_at: f64,
}

/// Namespace-level statistics.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct NamespaceStats {
    pub namespace: String,
    pub total_workflows: i64,
    pub running: i64,
    pub pending: i64,
    pub completed: i64,
    pub failed: i64,
    pub schedules: i64,
    pub workers: i64,
}

/// Task queue statistics.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct QueueStats {
    pub queue: String,
    pub pending_activities: i64,
    pub running_activities: i64,
    pub workers: i64,
}
