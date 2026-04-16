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

        // Phase 9: a freshly-started workflow has new events (WorkflowStarted)
        // that need a worker to replay against — make it dispatchable.
        self.store.mark_workflow_dispatchable(workflow_id).await?;

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

                // Two-phase cancel:
                //   1. Append WorkflowCancelRequested + mark dispatchable.
                //      The next worker replay sees the request, raises a
                //      cancellation error inside the handler, and submits
                //      a CancelWorkflow command.
                //   2. CancelWorkflow command flips status to CANCELLED,
                //      cancels pending activities/timers, appends
                //      WorkflowCancelled.
                //
                // We cancel pending activities + timers up-front too so a
                // worker that's about to claim them sees CANCELLED instead.
                self.store.cancel_pending_activities(id).await?;
                self.store.cancel_pending_timers(id).await?;

                let seq = self.store.get_event_count(id).await? as i32 + 1;
                self.store
                    .append_event(&WorkflowEvent {
                        id: None,
                        workflow_id: id.to_string(),
                        seq,
                        event_type: "WorkflowCancelRequested".to_string(),
                        payload: None,
                        timestamp: timestamp_now(),
                    })
                    .await?;

                self.store.mark_workflow_dispatchable(id).await?;

                // Propagate cancellation to all child workflows
                let children = self.store.list_child_workflows(id).await?;
                for child in children {
                    Box::pin(self.cancel_workflow(&child.id)).await?;
                }

                // For workflows that have NO worker registered (or no
                // handler running), cancellation would never complete.
                // Fall back: if the workflow has no events past
                // WorkflowStarted (handler hasn't actually run yet, e.g.
                // PENDING with no claim), finalise immediately.
                if matches!(status, WorkflowStatus::Pending) {
                    self.finalise_cancellation(id).await?;
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
        // Parse the incoming payload string back to a JSON value so the
        // event payload nests cleanly (otherwise the recorded payload is
        // a stringified JSON-inside-JSON and Lua workers would have to
        // double-decode).
        let payload_value: serde_json::Value = payload
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(serde_json::Value::Null);
        self.store
            .append_event(&WorkflowEvent {
                id: None,
                workflow_id: workflow_id.to_string(),
                seq,
                event_type: "SignalReceived".to_string(),
                payload: Some(
                    serde_json::json!({ "signal": name, "payload": payload_value }).to_string(),
                ),
                timestamp: now,
            })
            .await?;

        // Phase 9: a workflow waiting on this signal needs to be re-dispatched
        // so the worker can replay and notice the signal in history.
        self.store.mark_workflow_dispatchable(workflow_id).await?;

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

    /// Mark a successfully-executed activity complete and append an
    /// `ActivityCompleted` event to the workflow event log so a replaying
    /// workflow can pick up the cached result.
    ///
    /// `failed=true` is preserved for legacy callers that go straight
    /// through complete with a non-retry path; new code should call
    /// [`Engine::fail_activity`] instead so retry policy is honored.
    pub async fn complete_activity(
        &self,
        id: i64,
        result: Option<&str>,
        error: Option<&str>,
        failed: bool,
    ) -> Result<()> {
        self.store.complete_activity(id, result, error, failed).await?;

        // Look up the activity so we can attribute the event correctly
        let act = match self.store.get_activity(id).await? {
            Some(a) => a,
            None => return Ok(()),
        };

        let event_type = if failed {
            "ActivityFailed"
        } else {
            "ActivityCompleted"
        };
        let payload = serde_json::json!({
            "activity_id": id,
            "activity_seq": act.seq,
            "name": act.name,
            "result": result.and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
            "error": error,
        });
        let event_seq = self.store.get_event_count(&act.workflow_id).await? as i32 + 1;
        let workflow_id = act.workflow_id.clone();
        self.store
            .append_event(&WorkflowEvent {
                id: None,
                workflow_id: act.workflow_id,
                seq: event_seq,
                event_type: event_type.to_string(),
                payload: Some(payload.to_string()),
                timestamp: timestamp_now(),
            })
            .await?;
        // Phase 9: the workflow has a new event the handler needs to see —
        // wake the workflow task back up.
        self.store.mark_workflow_dispatchable(&workflow_id).await?;
        Ok(())
    }

    /// Fail an activity, honoring its retry policy.
    ///
    /// If `attempt < max_attempts`, the activity is re-queued with
    /// exponential backoff (`initial_interval_secs * backoff_coefficient^(attempt-1)`)
    /// and `attempt` is incremented. **No event is appended** — retries
    /// are an internal-engine concern, not workflow-visible.
    ///
    /// If `attempt >= max_attempts`, the activity is permanently FAILED
    /// and an `ActivityFailed` event is appended so the workflow can react.
    pub async fn fail_activity(&self, id: i64, error: &str) -> Result<()> {
        let act = match self.store.get_activity(id).await? {
            Some(a) => a,
            None => return Ok(()),
        };

        if act.attempt < act.max_attempts {
            // Compute exponential backoff: interval * coefficient^(attempt-1)
            let backoff = act.initial_interval_secs
                * act.backoff_coefficient.powi(act.attempt - 1);
            let next_scheduled_at = timestamp_now() + backoff;
            self.store
                .requeue_activity_for_retry(id, act.attempt + 1, next_scheduled_at)
                .await?;
            return Ok(());
        }

        // Out of retries — mark FAILED and surface to the workflow
        self.store
            .complete_activity(id, None, Some(error), true)
            .await?;

        let event_seq = self.store.get_event_count(&act.workflow_id).await? as i32 + 1;
        let workflow_id = act.workflow_id.clone();
        self.store
            .append_event(&WorkflowEvent {
                id: None,
                workflow_id: act.workflow_id,
                seq: event_seq,
                event_type: "ActivityFailed".to_string(),
                payload: Some(
                    serde_json::json!({
                        "activity_id": id,
                        "activity_seq": act.seq,
                        "name": act.name,
                        "error": error,
                        "final_attempt": act.attempt,
                    })
                    .to_string(),
                ),
                timestamp: timestamp_now(),
            })
            .await?;
        // Wake the workflow task — handler needs to see the failure.
        self.store.mark_workflow_dispatchable(&workflow_id).await?;
        Ok(())
    }

    // ── Workflow-task dispatch (Phase 9) ────────────────────

    /// Claim a dispatchable workflow task on a queue. Returns the workflow
    /// record + full event history so the worker can replay the handler
    /// deterministically. Atomic — multiple workers polling the same queue
    /// will each get a different task or None.
    pub async fn claim_workflow_task(
        &self,
        task_queue: &str,
        worker_id: &str,
    ) -> Result<Option<(WorkflowRecord, Vec<WorkflowEvent>)>> {
        let Some(mut wf) = self
            .store
            .claim_workflow_task(task_queue, worker_id)
            .await?
        else {
            return Ok(None);
        };
        // Once a worker is processing the workflow it's RUNNING — even if
        // it ultimately just yields and pauses on a signal/timer. PENDING
        // means "no worker has touched this yet."
        if wf.status == "PENDING" {
            self.store
                .update_workflow_status(&wf.id, WorkflowStatus::Running, None, None)
                .await?;
            wf.status = "RUNNING".to_string();
        }
        let history = self.store.list_events(&wf.id).await?;
        Ok(Some((wf, history)))
    }

    /// Submit a worker's batch of commands for a workflow it claimed.
    /// Each command produces durable events / rows transactionally and
    /// the dispatch lease is released on return.
    ///
    /// Supported command types:
    /// - `ScheduleActivity` { seq, name, task_queue, input?, max_attempts?, ... }
    /// - `CompleteWorkflow` { result }
    /// - `FailWorkflow`     { error }
    pub async fn submit_workflow_commands(
        &self,
        workflow_id: &str,
        worker_id: &str,
        commands: &[serde_json::Value],
    ) -> Result<()> {
        for cmd in commands {
            let cmd_type = cmd.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match cmd_type {
                "ScheduleActivity" => {
                    let seq = cmd.get("seq").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                    let name = cmd.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let queue = cmd
                        .get("task_queue")
                        .and_then(|v| v.as_str())
                        .unwrap_or("default");
                    let input = cmd.get("input").map(|v| v.to_string());
                    let opts = ScheduleActivityOpts {
                        max_attempts: cmd
                            .get("max_attempts")
                            .and_then(|v| v.as_i64())
                            .map(|n| n as i32),
                        initial_interval_secs: cmd
                            .get("initial_interval_secs")
                            .and_then(|v| v.as_f64()),
                        backoff_coefficient: cmd
                            .get("backoff_coefficient")
                            .and_then(|v| v.as_f64()),
                        start_to_close_secs: cmd
                            .get("start_to_close_secs")
                            .and_then(|v| v.as_f64()),
                        heartbeat_timeout_secs: cmd
                            .get("heartbeat_timeout_secs")
                            .and_then(|v| v.as_f64()),
                    };
                    self.schedule_activity(
                        workflow_id,
                        seq,
                        name,
                        input.as_deref(),
                        queue,
                        opts,
                    )
                    .await?;
                }
                "CancelWorkflow" => {
                    // Worker acknowledged a cancellation — finalise.
                    self.finalise_cancellation(workflow_id).await?;
                }
                "WaitForSignal" => {
                    // No engine-side state to write — the workflow has paused
                    // and will be re-dispatched when a matching signal arrives.
                    // Releasing the lease (below) is enough; record the wait
                    // intent for the dashboard / debugging.
                    let signal_name =
                        cmd.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let event_seq =
                        self.store.get_event_count(workflow_id).await? as i32 + 1;
                    self.store
                        .append_event(&WorkflowEvent {
                            id: None,
                            workflow_id: workflow_id.to_string(),
                            seq: event_seq,
                            event_type: "WorkflowAwaitingSignal".to_string(),
                            payload: Some(
                                serde_json::json!({ "signal": signal_name }).to_string(),
                            ),
                            timestamp: timestamp_now(),
                        })
                        .await?;
                }
                "ScheduleTimer" => {
                    let seq = cmd.get("seq").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                    let duration = cmd
                        .get("duration_secs")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
                    self.schedule_timer(workflow_id, seq, duration).await?;
                }
                "CompleteWorkflow" => {
                    let result = cmd.get("result").map(|v| v.to_string());
                    self.complete_workflow(workflow_id, result.as_deref()).await?;
                }
                "FailWorkflow" => {
                    let error = cmd
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("workflow handler raised an error");
                    self.fail_workflow(workflow_id, error).await?;
                }
                other => {
                    tracing::warn!("submit_workflow_commands: unknown command type {other:?}");
                }
            }
        }

        self.store
            .release_workflow_task(workflow_id, worker_id)
            .await?;
        Ok(())
    }

    /// Schedule a durable timer for a workflow.
    ///
    /// Idempotent on `(workflow_id, seq)` — a workflow that yields the same
    /// `ScheduleTimer{seq=N}` on retry will reuse the existing timer, not
    /// schedule a second one. This is the timer counterpart to
    /// `schedule_activity`'s replay-safe behaviour.
    ///
    /// On the first call:
    /// - inserts a row in `workflow_timers` with `fire_at = now + duration`
    /// - appends a `TimerScheduled` event so the worker can replay and
    ///   know it's been scheduled (otherwise replays would yield it again)
    pub async fn schedule_timer(
        &self,
        workflow_id: &str,
        seq: i32,
        duration_secs: f64,
    ) -> Result<WorkflowTimer> {
        if let Some(existing) = self
            .store
            .get_timer_by_workflow_seq(workflow_id, seq)
            .await?
        {
            return Ok(existing);
        }

        let now = timestamp_now();
        let mut timer = WorkflowTimer {
            id: None,
            workflow_id: workflow_id.to_string(),
            seq,
            fire_at: now + duration_secs,
            fired: false,
        };
        let id = self.store.create_timer(&timer).await?;
        timer.id = Some(id);

        let event_seq = self.store.get_event_count(workflow_id).await? as i32 + 1;
        self.store
            .append_event(&WorkflowEvent {
                id: None,
                workflow_id: workflow_id.to_string(),
                seq: event_seq,
                event_type: "TimerScheduled".to_string(),
                payload: Some(
                    serde_json::json!({
                        "timer_id": id,
                        "timer_seq": seq,
                        "fire_at": timer.fire_at,
                        "duration_secs": duration_secs,
                    })
                    .to_string(),
                ),
                timestamp: now,
            })
            .await?;

        Ok(timer)
    }

    /// Finalise a cancellation: flips status to CANCELLED and appends the
    /// terminal WorkflowCancelled event. Called by the CancelWorkflow
    /// command handler (worker acknowledged cancel) and by cancel_workflow
    /// directly when the workflow has no worker yet.
    pub async fn finalise_cancellation(&self, workflow_id: &str) -> Result<()> {
        // Avoid double-finalising
        if let Some(wf) = self.store.get_workflow(workflow_id).await? {
            if wf.status == "CANCELLED" {
                return Ok(());
            }
        }
        self.store
            .update_workflow_status(workflow_id, WorkflowStatus::Cancelled, None, None)
            .await?;
        let event_seq = self.store.get_event_count(workflow_id).await? as i32 + 1;
        self.store
            .append_event(&WorkflowEvent {
                id: None,
                workflow_id: workflow_id.to_string(),
                seq: event_seq,
                event_type: "WorkflowCancelled".to_string(),
                payload: None,
                timestamp: timestamp_now(),
            })
            .await?;
        Ok(())
    }

    /// Mark a workflow COMPLETED with a result + append WorkflowCompleted event.
    pub async fn complete_workflow(&self, workflow_id: &str, result: Option<&str>) -> Result<()> {
        self.store
            .update_workflow_status(workflow_id, WorkflowStatus::Completed, result, None)
            .await?;
        let event_seq = self.store.get_event_count(workflow_id).await? as i32 + 1;
        self.store
            .append_event(&WorkflowEvent {
                id: None,
                workflow_id: workflow_id.to_string(),
                seq: event_seq,
                event_type: "WorkflowCompleted".to_string(),
                payload: result.map(String::from),
                timestamp: timestamp_now(),
            })
            .await?;
        Ok(())
    }

    /// Mark a workflow FAILED with an error + append WorkflowFailed event.
    pub async fn fail_workflow(&self, workflow_id: &str, error: &str) -> Result<()> {
        self.store
            .update_workflow_status(workflow_id, WorkflowStatus::Failed, None, Some(error))
            .await?;
        let event_seq = self.store.get_event_count(workflow_id).await? as i32 + 1;
        self.store
            .append_event(&WorkflowEvent {
                id: None,
                workflow_id: workflow_id.to_string(),
                seq: event_seq,
                event_type: "WorkflowFailed".to_string(),
                payload: Some(serde_json::json!({"error": error}).to_string()),
                timestamp: timestamp_now(),
            })
            .await?;
        Ok(())
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
