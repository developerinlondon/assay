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
        workflow_type: &str,
        workflow_id: &str,
        input: Option<&str>,
        task_queue: &str,
    ) -> Result<WorkflowRecord> {
        let now = timestamp_now();
        let run_id = format!("run-{workflow_id}-{}", now as u64);

        let wf = WorkflowRecord {
            id: workflow_id.to_string(),
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
        status: Option<WorkflowStatus>,
        workflow_type: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<WorkflowRecord>> {
        self.store
            .list_workflows(status, workflow_type, limit, offset)
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

    pub async fn list_workers(&self) -> Result<Vec<WorkflowWorker>> {
        self.store.list_workers().await
    }

    // ── Task Operations (for worker polling) ────────────────

    pub async fn claim_activity(
        &self,
        task_queue: &str,
        worker_id: &str,
    ) -> Result<Option<WorkflowActivity>> {
        self.store.claim_activity(task_queue, worker_id).await
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

    pub async fn list_schedules(&self) -> Result<Vec<WorkflowSchedule>> {
        self.store.list_schedules().await
    }

    pub async fn get_schedule(&self, name: &str) -> Result<Option<WorkflowSchedule>> {
        self.store.get_schedule(name).await
    }

    pub async fn delete_schedule(&self, name: &str) -> Result<bool> {
        self.store.delete_schedule(name).await
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
