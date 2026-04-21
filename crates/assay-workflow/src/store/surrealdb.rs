//! SurrealDB backend for `WorkflowStore`.
//!
//! `connect_full` is a stub that returns an error until Phase 3 (plan 12b
//! Task 3.1) lands the real implementation. The parametrised test harness
//! surfaces this as a deliberate test failure on the Surreal case.

use std::future::Future;

use assay_core::store::WorkflowStore;
use assay_core::types::*;

pub struct SurrealDbStore {
    _priv: (),
}

impl SurrealDbStore {
    /// Connect to a SurrealDB instance.
    ///
    /// **Not yet implemented** — stubs until plan 12b Task 3.1.
    pub async fn connect_full(
        _url: &str,
        _namespace: &str,
        _database: &str,
        _username: Option<&str>,
        _password: Option<&str>,
    ) -> anyhow::Result<Self> {
        anyhow::bail!("SurrealDbStore::connect_full — not yet implemented (Phase 3)")
    }
}

impl WorkflowStore for SurrealDbStore {
    fn create_namespace(&self, _name: &str) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { todo!("Task 3.2") }
    }

    fn list_namespaces(
        &self,
    ) -> impl Future<Output = anyhow::Result<Vec<NamespaceRecord>>> + Send {
        async { todo!("Task 3.2") }
    }

    fn delete_namespace(&self, _name: &str) -> impl Future<Output = anyhow::Result<bool>> + Send {
        async { todo!("Task 3.2") }
    }

    fn get_namespace_stats(
        &self,
        _namespace: &str,
    ) -> impl Future<Output = anyhow::Result<NamespaceStats>> + Send {
        async { todo!("Task 3.2") }
    }

    fn create_workflow(
        &self,
        _workflow: &WorkflowRecord,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { todo!("Task 3.3") }
    }

    fn get_workflow(
        &self,
        _id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowRecord>>> + Send {
        async { todo!("Task 3.3") }
    }

    fn list_workflows(
        &self,
        _namespace: &str,
        _status: Option<WorkflowStatus>,
        _workflow_type: Option<&str>,
        _search_attrs_filter: Option<&str>,
        _limit: i64,
        _offset: i64,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowRecord>>> + Send {
        async { todo!("Task 3.3") }
    }

    fn list_archivable_workflows(
        &self,
        _cutoff: f64,
        _limit: i64,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowRecord>>> + Send {
        async { todo!("Task 3.11") }
    }

    fn mark_archived_and_purge(
        &self,
        _workflow_id: &str,
        _archive_uri: &str,
        _archived_at: f64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { todo!("Task 3.11") }
    }

    fn upsert_search_attributes(
        &self,
        _workflow_id: &str,
        _patch_json: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { todo!("Task 3.5") }
    }

    fn update_workflow_status(
        &self,
        _id: &str,
        _status: WorkflowStatus,
        _result: Option<&str>,
        _error: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { todo!("Task 3.3") }
    }

    fn claim_workflow(
        &self,
        _id: &str,
        _worker_id: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send {
        async { todo!("Task 3.3") }
    }

    fn mark_workflow_dispatchable(
        &self,
        _workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { todo!("Task 3.3") }
    }

    fn claim_workflow_task(
        &self,
        _task_queue: &str,
        _worker_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowRecord>>> + Send {
        async { todo!("Task 3.3") }
    }

    fn release_workflow_task(
        &self,
        _workflow_id: &str,
        _worker_id: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { todo!("Task 3.3") }
    }

    fn release_stale_dispatch_leases(
        &self,
        _now: f64,
        _timeout_secs: f64,
    ) -> impl Future<Output = anyhow::Result<u64>> + Send {
        async { todo!("Task 3.3") }
    }

    fn append_event(
        &self,
        _event: &WorkflowEvent,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send {
        async { todo!("Task 3.4") }
    }

    fn list_events(
        &self,
        _workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowEvent>>> + Send {
        async { todo!("Task 3.4") }
    }

    fn get_event_count(
        &self,
        _workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send {
        async { todo!("Task 3.4") }
    }

    fn create_activity(
        &self,
        _activity: &WorkflowActivity,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send {
        async { todo!("Task 3.6") }
    }

    fn get_activity(
        &self,
        _id: i64,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowActivity>>> + Send {
        async { todo!("Task 3.6") }
    }

    fn get_activity_by_workflow_seq(
        &self,
        _workflow_id: &str,
        _seq: i32,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowActivity>>> + Send {
        async { todo!("Task 3.6") }
    }

    fn claim_activity(
        &self,
        _task_queue: &str,
        _worker_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowActivity>>> + Send {
        async { todo!("Task 3.6") }
    }

    fn requeue_activity_for_retry(
        &self,
        _id: i64,
        _next_attempt: i32,
        _next_scheduled_at: f64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { todo!("Task 3.6") }
    }

    fn complete_activity(
        &self,
        _id: i64,
        _result: Option<&str>,
        _error: Option<&str>,
        _failed: bool,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { todo!("Task 3.6") }
    }

    fn heartbeat_activity(
        &self,
        _id: i64,
        _details: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { todo!("Task 3.6") }
    }

    fn get_timed_out_activities(
        &self,
        _now: f64,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowActivity>>> + Send {
        async { todo!("Task 3.6") }
    }

    fn cancel_pending_activities(
        &self,
        _workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<u64>> + Send {
        async { todo!("Task 3.6") }
    }

    fn cancel_pending_timers(
        &self,
        _workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<u64>> + Send {
        async { todo!("Task 3.7") }
    }

    fn create_timer(
        &self,
        _timer: &WorkflowTimer,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send {
        async { todo!("Task 3.7") }
    }

    fn get_timer_by_workflow_seq(
        &self,
        _workflow_id: &str,
        _seq: i32,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowTimer>>> + Send {
        async { todo!("Task 3.7") }
    }

    fn fire_due_timers(
        &self,
        _now: f64,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowTimer>>> + Send {
        async { todo!("Task 3.7") }
    }

    fn send_signal(
        &self,
        _signal: &WorkflowSignal,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send {
        async { todo!("Task 3.8") }
    }

    fn consume_signals(
        &self,
        _workflow_id: &str,
        _name: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowSignal>>> + Send {
        async { todo!("Task 3.8") }
    }

    fn create_schedule(
        &self,
        _schedule: &WorkflowSchedule,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { todo!("Task 3.9") }
    }

    fn get_schedule(
        &self,
        _namespace: &str,
        _name: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowSchedule>>> + Send {
        async { todo!("Task 3.9") }
    }

    fn list_schedules(
        &self,
        _namespace: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowSchedule>>> + Send {
        async { todo!("Task 3.9") }
    }

    fn update_schedule_last_run(
        &self,
        _namespace: &str,
        _name: &str,
        _last_run_at: f64,
        _next_run_at: f64,
        _workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { todo!("Task 3.9") }
    }

    fn delete_schedule(
        &self,
        _namespace: &str,
        _name: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send {
        async { todo!("Task 3.9") }
    }

    fn update_schedule(
        &self,
        _namespace: &str,
        _name: &str,
        _patch: &SchedulePatch,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowSchedule>>> + Send {
        async { todo!("Task 3.9") }
    }

    fn set_schedule_paused(
        &self,
        _namespace: &str,
        _name: &str,
        _paused: bool,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowSchedule>>> + Send {
        async { todo!("Task 3.9") }
    }

    fn register_worker(
        &self,
        _worker: &WorkflowWorker,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { todo!("Task 3.12") }
    }

    fn heartbeat_worker(
        &self,
        _id: &str,
        _now: f64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { todo!("Task 3.12") }
    }

    fn list_workers(
        &self,
        _namespace: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowWorker>>> + Send {
        async { todo!("Task 3.12") }
    }

    fn remove_dead_workers(
        &self,
        _cutoff: f64,
    ) -> impl Future<Output = anyhow::Result<Vec<String>>> + Send {
        async { todo!("Task 3.12") }
    }

    fn create_api_key(
        &self,
        _key_hash: &str,
        _prefix: &str,
        _label: Option<&str>,
        _created_at: f64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { todo!("Task 3.13") }
    }

    fn validate_api_key(
        &self,
        _key_hash: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send {
        async { todo!("Task 3.13") }
    }

    fn list_api_keys(&self) -> impl Future<Output = anyhow::Result<Vec<ApiKeyRecord>>> + Send {
        async { todo!("Task 3.13") }
    }

    fn revoke_api_key(
        &self,
        _prefix: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send {
        async { todo!("Task 3.13") }
    }

    fn api_keys_empty(&self) -> impl Future<Output = anyhow::Result<bool>> + Send {
        async { todo!("Task 3.13") }
    }

    fn get_api_key_by_label(
        &self,
        _label: &str,
    ) -> impl Future<Output = anyhow::Result<Option<ApiKeyRecord>>> + Send {
        async { todo!("Task 3.13") }
    }

    fn list_child_workflows(
        &self,
        _parent_id: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowRecord>>> + Send {
        async { todo!("Task 3.14") }
    }

    fn create_snapshot(
        &self,
        _workflow_id: &str,
        _event_seq: i32,
        _state_json: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { todo!("Task 3.10") }
    }

    fn get_latest_snapshot(
        &self,
        _workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowSnapshot>>> + Send {
        async { todo!("Task 3.10") }
    }

    fn get_queue_stats(
        &self,
        _namespace: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<QueueStats>>> + Send {
        async { todo!("Task 3.14") }
    }

    fn try_acquire_scheduler_lock(
        &self,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send {
        async { todo!("Task 3.15") }
    }

    fn subscribe_runnable(
        &self,
        _namespace: &str,
    ) -> impl futures_core::Stream<Item = String> + Send + '_ {
        futures_util::stream::empty()
    }

    fn subscribe_tasks<'a>(
        &'a self,
        _queue_names: &'a [&'a str],
    ) -> impl futures_core::Stream<Item = String> + Send + 'a {
        futures_util::stream::empty()
    }
}
