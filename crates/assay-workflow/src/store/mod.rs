pub mod sqlite;

use std::future::Future;

use crate::types::*;

/// Core storage trait for the workflow engine.
///
/// All database access goes through this trait. The engine, API, scheduler,
/// and health monitor depend only on `WorkflowStore`, never on a concrete
/// database implementation.
///
/// All methods return `Send` futures so they can be used from `tokio::spawn`.
pub trait WorkflowStore: Send + Sync + 'static {
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
        status: Option<WorkflowStatus>,
        workflow_type: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowRecord>>> + Send;

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

    fn claim_activity(
        &self,
        task_queue: &str,
        worker_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowActivity>>> + Send;

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

    fn create_timer(
        &self,
        timer: &WorkflowTimer,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send;

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
        name: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowSchedule>>> + Send;

    fn list_schedules(&self) -> impl Future<Output = anyhow::Result<Vec<WorkflowSchedule>>> + Send;

    fn update_schedule_last_run(
        &self,
        name: &str,
        last_run_at: f64,
        next_run_at: f64,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn delete_schedule(
        &self,
        name: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;

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

    fn list_workers(&self) -> impl Future<Output = anyhow::Result<Vec<WorkflowWorker>>> + Send;

    fn remove_dead_workers(
        &self,
        cutoff: f64,
    ) -> impl Future<Output = anyhow::Result<Vec<String>>> + Send;
}
