use std::sync::Arc;

use anyhow::Result;
use tokio::task::JoinHandle;
use tracing::info;

use crate::health;
use crate::scheduler;
use crate::store::WorkflowStore;
use crate::timers;
use crate::types::*;

/// The workflow engine. Owns the store and manages background tasks
/// (scheduler, timer poller, health monitor).
///
/// The API layer holds an `Arc<Engine<S>>` and delegates all operations here.
pub struct Engine<S: WorkflowStore> {
    store: Arc<S>,
    _scheduler: JoinHandle<()>,
    _timer_poller: JoinHandle<()>,
    _health_monitor: JoinHandle<()>,
}

impl<S: WorkflowStore> Engine<S> {
    /// Start the engine with all background tasks.
    pub fn start(store: S) -> Self {
        let store = Arc::new(store);

        let _scheduler = tokio::spawn(scheduler::run_scheduler(Arc::clone(&store)));
        let _timer_poller = tokio::spawn(timers::run_timer_poller(Arc::clone(&store)));
        let _health_monitor = tokio::spawn(health::run_health_monitor(Arc::clone(&store)));

        info!("Workflow engine started");

        Self {
            store,
            _scheduler,
            _timer_poller,
            _health_monitor,
        }
    }

    /// Access the underlying store (for the API layer).
    pub fn store(&self) -> &S {
        &self.store
    }

    // ── Workflow Operations ─────────────────────────────────

    pub async fn start_workflow(
        &self,
        namespace: &str,
        workflow_type: &str,
        workflow_id: &str,
        input: Option<&str>,
        task_queue: &str,
    ) -> Result<WorkflowRecord> {
        let now = timestamp_now();
        let run_id = format!("run-{workflow_id}-{}", now as u64);

        let wf = WorkflowRecord {
            id: workflow_id.to_string(),
            namespace: namespace.to_string(),
            run_id,
            workflow_type: workflow_type.to_string(),
            task_queue: task_queue.to_string(),
            status: "PENDING".to_string(),
            input: input.map(String::from),
            result: None,
            error: None,
            parent_id: None,
            claimed_by: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        };

        self.store.create_workflow(&wf).await?;

        self.store
            .append_event(&WorkflowEvent {
                id: None,
                workflow_id: workflow_id.to_string(),
                seq: 1,
                event_type: "WorkflowStarted".to_string(),
                payload: input.map(String::from),
                timestamp: now,
            })
            .await?;

        Ok(wf)
    }

    pub async fn get_workflow(&self, id: &str) -> Result<Option<WorkflowRecord>> {
        self.store.get_workflow(id).await
    }

    pub async fn list_workflows(
        &self,
        namespace: &str,
        status: Option<WorkflowStatus>,
        workflow_type: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<WorkflowRecord>> {
        self.store
            .list_workflows(namespace, status, workflow_type, limit, offset)
            .await
    }

    pub async fn cancel_workflow(&self, id: &str) -> Result<bool> {
        let wf = self.store.get_workflow(id).await?;
        match wf {
            None => Ok(false),
            Some(wf) => {
                let status = WorkflowStatus::from_str(&wf.status)
                    .map_err(|e| anyhow::anyhow!(e))?;
                if status.is_terminal() {
                    return Ok(false);
                }

                self.store
                    .update_workflow_status(id, WorkflowStatus::Cancelled, None, None)
                    .await?;

                let seq = self.store.get_event_count(id).await? as i32 + 1;
                self.store
                    .append_event(&WorkflowEvent {
                        id: None,
                        workflow_id: id.to_string(),
                        seq,
                        event_type: "WorkflowCancelled".to_string(),
                        payload: None,
                        timestamp: timestamp_now(),
                    })
                    .await?;

                // Propagate cancellation to all child workflows
                let children = self.store.list_child_workflows(id).await?;
                for child in children {
                    // Recursive cancellation — Box::pin for async recursion
                    Box::pin(self.cancel_workflow(&child.id)).await?;
                }

                Ok(true)
            }
        }
    }

    pub async fn terminate_workflow(&self, id: &str, reason: Option<&str>) -> Result<bool> {
        let wf = self.store.get_workflow(id).await?;
        match wf {
            None => Ok(false),
            Some(wf) => {
                let status = WorkflowStatus::from_str(&wf.status)
                    .map_err(|e| anyhow::anyhow!(e))?;
                if status.is_terminal() {
                    return Ok(false);
                }

                self.store
                    .update_workflow_status(
                        id,
                        WorkflowStatus::Failed,
                        None,
                        Some(reason.unwrap_or("terminated")),
                    )
                    .await?;

                Ok(true)
            }
        }
    }

    // ── Signal Operations ───────────────────────────────────

    pub async fn send_signal(
        &self,
        workflow_id: &str,
        name: &str,
        payload: Option<&str>,
    ) -> Result<()> {
        let now = timestamp_now();

        self.store
            .send_signal(&WorkflowSignal {
                id: None,
                workflow_id: workflow_id.to_string(),
                name: name.to_string(),
                payload: payload.map(String::from),
                consumed: false,
                received_at: now,
            })
            .await?;

        let seq = self.store.get_event_count(workflow_id).await? as i32 + 1;
        self.store
            .append_event(&WorkflowEvent {
                id: None,
                workflow_id: workflow_id.to_string(),
                seq,
                event_type: "SignalReceived".to_string(),
                payload: Some(
                    serde_json::json!({ "signal": name, "payload": payload }).to_string(),
                ),
                timestamp: now,
            })
            .await?;

        Ok(())
    }

    // ── Event History ───────────────────────────────────────

    pub async fn get_events(&self, workflow_id: &str) -> Result<Vec<WorkflowEvent>> {
        self.store.list_events(workflow_id).await
    }

    // ── Worker Operations ───────────────────────────────────

    pub async fn register_worker(&self, worker: &WorkflowWorker) -> Result<()> {
        self.store.register_worker(worker).await
    }

    pub async fn heartbeat_worker(&self, id: &str) -> Result<()> {
        self.store.heartbeat_worker(id, timestamp_now()).await
    }

    pub async fn list_workers(&self, namespace: &str) -> Result<Vec<WorkflowWorker>> {
        self.store.list_workers(namespace).await
    }

    // ── Task Operations (for worker polling) ────────────────

    /// Schedule an activity within a workflow.
    ///
    /// Idempotent on `(workflow_id, seq)` — if an activity with this sequence
    /// number already exists for the workflow, returns its id without
    /// creating a duplicate row or duplicate `ActivityScheduled` event. This
    /// is essential for deterministic replay: a worker can re-run the
    /// workflow function and call `schedule_activity(seq=1, ...)` repeatedly
    /// without producing side effects.
    ///
    /// On the first call for a `seq`:
    /// - inserts a row in `workflow_activities` with status `PENDING`
    /// - appends an `ActivityScheduled` event to the workflow event log
    /// - if the workflow is still `PENDING`, transitions it to `RUNNING`
    pub async fn schedule_activity(
        &self,
        workflow_id: &str,
        seq: i32,
        name: &str,
        input: Option<&str>,
        task_queue: &str,
        opts: ScheduleActivityOpts,
    ) -> Result<WorkflowActivity> {
        // Idempotency: short-circuit if (workflow_id, seq) already exists.
        if let Some(existing) = self
            .store
            .get_activity_by_workflow_seq(workflow_id, seq)
            .await?
        {
            return Ok(existing);
        }

        let now = timestamp_now();
        let mut act = WorkflowActivity {
            id: None,
            workflow_id: workflow_id.to_string(),
            seq,
            name: name.to_string(),
            task_queue: task_queue.to_string(),
            input: input.map(String::from),
            status: "PENDING".to_string(),
            result: None,
            error: None,
            attempt: 1,
            max_attempts: opts.max_attempts.unwrap_or(3),
            initial_interval_secs: opts.initial_interval_secs.unwrap_or(1.0),
            backoff_coefficient: opts.backoff_coefficient.unwrap_or(2.0),
            start_to_close_secs: opts.start_to_close_secs.unwrap_or(300.0),
            heartbeat_timeout_secs: opts.heartbeat_timeout_secs,
            claimed_by: None,
            scheduled_at: now,
            started_at: None,
            completed_at: None,
            last_heartbeat: None,
        };

        let id = self.store.create_activity(&act).await?;
        act.id = Some(id);

        // Append ActivityScheduled event with the activity's seq
        let event_seq = self.store.get_event_count(workflow_id).await? as i32 + 1;
        self.store
            .append_event(&WorkflowEvent {
                id: None,
                workflow_id: workflow_id.to_string(),
                seq: event_seq,
                event_type: "ActivityScheduled".to_string(),
                payload: Some(
                    serde_json::json!({
                        "activity_id": id,
                        "activity_seq": seq,
                        "name": name,
                        "task_queue": task_queue,
                        "input": input,
                    })
                    .to_string(),
                ),
                timestamp: now,
            })
            .await?;

        // Transition workflow from PENDING to RUNNING on first scheduled activity
        if let Some(wf) = self.store.get_workflow(workflow_id).await? {
            if wf.status == "PENDING" {
                self.store
                    .update_workflow_status(workflow_id, WorkflowStatus::Running, None, None)
                    .await?;
            }
        }

        Ok(act)
    }

    pub async fn claim_activity(
        &self,
        task_queue: &str,
        worker_id: &str,
    ) -> Result<Option<WorkflowActivity>> {
        self.store.claim_activity(task_queue, worker_id).await
    }

    pub async fn get_activity(&self, id: i64) -> Result<Option<WorkflowActivity>> {
        self.store.get_activity(id).await
    }

    pub async fn complete_activity(
        &self,
        id: i64,
        result: Option<&str>,
        error: Option<&str>,
        failed: bool,
    ) -> Result<()> {
        self.store.complete_activity(id, result, error, failed).await
    }

    pub async fn heartbeat_activity(&self, id: i64, details: Option<&str>) -> Result<()> {
        self.store.heartbeat_activity(id, details).await
    }

    // ── Schedule Operations ─────────────────────────────────

    pub async fn create_schedule(&self, schedule: &WorkflowSchedule) -> Result<()> {
        self.store.create_schedule(schedule).await
    }

    pub async fn list_schedules(&self, namespace: &str) -> Result<Vec<WorkflowSchedule>> {
        self.store.list_schedules(namespace).await
    }

    pub async fn get_schedule(&self, namespace: &str, name: &str) -> Result<Option<WorkflowSchedule>> {
        self.store.get_schedule(namespace, name).await
    }

    pub async fn delete_schedule(&self, namespace: &str, name: &str) -> Result<bool> {
        self.store.delete_schedule(namespace, name).await
    }

    // ── Namespace Operations ────────────────────────────────

    pub async fn create_namespace(&self, name: &str) -> Result<()> {
        self.store.create_namespace(name).await
    }

    pub async fn list_namespaces(&self) -> Result<Vec<crate::store::NamespaceRecord>> {
        self.store.list_namespaces().await
    }

    pub async fn delete_namespace(&self, name: &str) -> Result<bool> {
        self.store.delete_namespace(name).await
    }

    pub async fn get_namespace_stats(&self, namespace: &str) -> Result<crate::store::NamespaceStats> {
        self.store.get_namespace_stats(namespace).await
    }

    pub async fn get_queue_stats(&self, namespace: &str) -> Result<Vec<crate::store::QueueStats>> {
        self.store.get_queue_stats(namespace).await
    }

    // ── Child Workflow Operations ───────────────────────────

    pub async fn start_child_workflow(
        &self,
        namespace: &str,
        parent_id: &str,
        workflow_type: &str,
        workflow_id: &str,
        input: Option<&str>,
        task_queue: &str,
    ) -> Result<WorkflowRecord> {
        let now = timestamp_now();
        let run_id = format!("run-{workflow_id}-{}", now as u64);

        let wf = WorkflowRecord {
            id: workflow_id.to_string(),
            namespace: namespace.to_string(),
            run_id,
            workflow_type: workflow_type.to_string(),
            task_queue: task_queue.to_string(),
            status: "PENDING".to_string(),
            input: input.map(String::from),
            result: None,
            error: None,
            parent_id: Some(parent_id.to_string()),
            claimed_by: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        };

        self.store.create_workflow(&wf).await?;

        // Record events on both parent and child
        self.store
            .append_event(&WorkflowEvent {
                id: None,
                workflow_id: workflow_id.to_string(),
                seq: 1,
                event_type: "WorkflowStarted".to_string(),
                payload: input.map(String::from),
                timestamp: now,
            })
            .await?;

        let parent_seq = self.store.get_event_count(parent_id).await? as i32 + 1;
        self.store
            .append_event(&WorkflowEvent {
                id: None,
                workflow_id: parent_id.to_string(),
                seq: parent_seq,
                event_type: "ChildWorkflowStarted".to_string(),
                payload: Some(
                    serde_json::json!({
                        "child_workflow_id": workflow_id,
                        "workflow_type": workflow_type,
                    })
                    .to_string(),
                ),
                timestamp: now,
            })
            .await?;

        Ok(wf)
    }

    pub async fn list_child_workflows(
        &self,
        parent_id: &str,
    ) -> Result<Vec<WorkflowRecord>> {
        self.store.list_child_workflows(parent_id).await
    }

    // ── Continue-as-New ─────────────────────────────────────

    pub async fn continue_as_new(
        &self,
        workflow_id: &str,
        input: Option<&str>,
    ) -> Result<WorkflowRecord> {
        let old_wf = self
            .store
            .get_workflow(workflow_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("workflow not found: {workflow_id}"))?;

        // Complete the old workflow
        self.store
            .update_workflow_status(workflow_id, WorkflowStatus::Completed, None, None)
            .await?;

        // Start a new run with the same type, namespace, and queue
        let new_id = format!("{workflow_id}-continued-{}", timestamp_now() as u64);
        self.start_workflow(
            &old_wf.namespace,
            &old_wf.workflow_type,
            &new_id,
            input,
            &old_wf.task_queue,
        )
        .await
    }

    // ── Snapshots ───────────────────────────────────────────

    pub async fn create_snapshot(
        &self,
        workflow_id: &str,
        event_seq: i32,
        state_json: &str,
    ) -> Result<()> {
        self.store
            .create_snapshot(workflow_id, event_seq, state_json)
            .await
    }

    pub async fn get_latest_snapshot(
        &self,
        workflow_id: &str,
    ) -> Result<Option<WorkflowSnapshot>> {
        self.store.get_latest_snapshot(workflow_id).await
    }

    // ── Side Effects ────────────────────────────────────────

    pub async fn record_side_effect(
        &self,
        workflow_id: &str,
        value: &str,
    ) -> Result<()> {
        let now = timestamp_now();
        let seq = self.store.get_event_count(workflow_id).await? as i32 + 1;
        self.store
            .append_event(&WorkflowEvent {
                id: None,
                workflow_id: workflow_id.to_string(),
                seq,
                event_type: "SideEffectRecorded".to_string(),
                payload: Some(value.to_string()),
                timestamp: now,
            })
            .await?;
        Ok(())
    }
}

fn timestamp_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

// WorkflowStatus::from_str returns Result, re-export for convenience
use std::str::FromStr;
