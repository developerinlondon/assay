pub mod sqlite;

use crate::workflow::types::*;

/// Core storage trait for the workflow engine.
///
/// All database access goes through this trait. The engine, API, scheduler,
/// and health monitor depend only on `WorkflowStore`, never on a concrete
/// database implementation.
#[allow(async_fn_in_trait)]
pub trait WorkflowStore: Send + Sync + 'static {
    // ── Workflows ──────────────────────────────────────────

    async fn create_workflow(&self, workflow: &WorkflowRecord) -> anyhow::Result<()>;
    async fn get_workflow(&self, id: &str) -> anyhow::Result<Option<WorkflowRecord>>;
    async fn list_workflows(
        &self,
        status: Option<WorkflowStatus>,
        workflow_type: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<Vec<WorkflowRecord>>;
    async fn update_workflow_status(
        &self,
        id: &str,
        status: WorkflowStatus,
        result: Option<&str>,
        error: Option<&str>,
    ) -> anyhow::Result<()>;
    async fn claim_workflow(&self, id: &str, worker_id: &str) -> anyhow::Result<bool>;

    // ── Events ─────────────────────────────────────────────

    async fn append_event(&self, event: &WorkflowEvent) -> anyhow::Result<i64>;
    async fn list_events(&self, workflow_id: &str) -> anyhow::Result<Vec<WorkflowEvent>>;
    async fn get_event_count(&self, workflow_id: &str) -> anyhow::Result<i64>;

    // ── Activities ──────────────────────────────────────────

    async fn create_activity(&self, activity: &WorkflowActivity) -> anyhow::Result<i64>;
    async fn claim_activity(
        &self,
        task_queue: &str,
        worker_id: &str,
    ) -> anyhow::Result<Option<WorkflowActivity>>;
    async fn complete_activity(
        &self,
        id: i64,
        result: Option<&str>,
        error: Option<&str>,
        failed: bool,
    ) -> anyhow::Result<()>;
    async fn heartbeat_activity(&self, id: i64, details: Option<&str>) -> anyhow::Result<()>;
    async fn get_timed_out_activities(&self, now: f64) -> anyhow::Result<Vec<WorkflowActivity>>;

    // ── Timers ──────────────────────────────────────────────

    async fn create_timer(&self, timer: &WorkflowTimer) -> anyhow::Result<i64>;
    async fn fire_due_timers(&self, now: f64) -> anyhow::Result<Vec<WorkflowTimer>>;

    // ── Signals ─────────────────────────────────────────────

    async fn send_signal(&self, signal: &WorkflowSignal) -> anyhow::Result<i64>;
    async fn consume_signals(
        &self,
        workflow_id: &str,
        name: &str,
    ) -> anyhow::Result<Vec<WorkflowSignal>>;

    // ── Schedules ───────────────────────────────────────────

    async fn create_schedule(&self, schedule: &WorkflowSchedule) -> anyhow::Result<()>;
    async fn get_schedule(&self, name: &str) -> anyhow::Result<Option<WorkflowSchedule>>;
    async fn list_schedules(&self) -> anyhow::Result<Vec<WorkflowSchedule>>;
    async fn update_schedule_last_run(
        &self,
        name: &str,
        last_run_at: f64,
        next_run_at: f64,
        workflow_id: &str,
    ) -> anyhow::Result<()>;
    async fn delete_schedule(&self, name: &str) -> anyhow::Result<bool>;

    // ── Workers ─────────────────────────────────────────────

    async fn register_worker(&self, worker: &WorkflowWorker) -> anyhow::Result<()>;
    async fn heartbeat_worker(&self, id: &str, now: f64) -> anyhow::Result<()>;
    async fn list_workers(&self) -> anyhow::Result<Vec<WorkflowWorker>>;
    async fn remove_dead_workers(&self, cutoff: f64) -> anyhow::Result<Vec<String>>;
}
