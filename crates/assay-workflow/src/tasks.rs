//! Workflow-task dispatch and timers.

use anyhow::Result;

use crate::ctx::{timestamp_now, WorkflowCtx};
use crate::events::WorkflowBusEvent;
use crate::store::WorkflowStore;
use crate::types::*;

impl<S: WorkflowStore> WorkflowCtx<S> {
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
            // Live-update the dashboard so the row flips PENDING →
            // RUNNING without requiring F5. Expected lifecycle for
            // durable workflows is to start PENDING and advance once
            // a worker claims, so this emit completes the loop.
            self.emit(
                &wf.namespace,
                WorkflowBusEvent::WorkflowRunning {
                    workflow_id: wf.id.clone(),
                },
            )
            .await;
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
                    //
                    // When the command carries `timer_seq`, the wait is paired
                    // with a `ScheduleTimer` yielded in the same batch — the
                    // worker uses the timer_seq to pick the winner on replay
                    // (signal vs timeout). The engine stores the pairing on
                    // the event for observability only.
                    let signal_name =
                        cmd.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let timer_seq = cmd.get("timer_seq").and_then(|v| v.as_i64());
                    let payload = match timer_seq {
                        Some(ts) => serde_json::json!({
                            "signal": signal_name,
                            "timer_seq": ts,
                        }),
                        None => serde_json::json!({ "signal": signal_name }),
                    };
                    let event_seq =
                        self.store.get_event_count(workflow_id).await? as i32 + 1;
                    self.store
                        .append_event(&WorkflowEvent {
                            id: None,
                            workflow_id: workflow_id.to_string(),
                            seq: event_seq,
                            event_type: "WorkflowAwaitingSignal".to_string(),
                            payload: Some(payload.to_string()),
                            timestamp: timestamp_now(),
                        })
                        .await?;
                }
                "StartChildWorkflow" => {
                    let workflow_type = cmd
                        .get("workflow_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let child_id =
                        cmd.get("workflow_id").and_then(|v| v.as_str()).unwrap_or("");
                    let task_queue = cmd
                        .get("task_queue")
                        .and_then(|v| v.as_str())
                        .unwrap_or("default");
                    let input = cmd.get("input").map(|v| v.to_string());
                    // Determine the namespace from the parent workflow
                    let namespace = self
                        .store
                        .get_workflow(workflow_id)
                        .await?
                        .map(|wf| wf.namespace)
                        .unwrap_or_else(|| "main".to_string());

                    // Idempotent: if a workflow with this id already exists,
                    // skip creation (deterministic replay calls this command
                    // for the same child id on every re-run until the parent
                    // has the ChildWorkflowCompleted event).
                    if self.store.get_workflow(child_id).await?.is_none() {
                        self.start_child_workflow(
                            &namespace,
                            workflow_id,
                            workflow_type,
                            child_id,
                            input.as_deref(),
                            task_queue,
                        )
                        .await?;
                        // Make the child immediately dispatchable so a worker
                        // picks it up; emit WorkflowNeedsDispatch via the bus.
                        self.mark_and_emit_needs_dispatch(child_id).await?;
                    }
                }
                "RecordSideEffect" => {
                    let seq = cmd.get("seq").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                    let name = cmd.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let value =
                        cmd.get("value").cloned().unwrap_or(serde_json::Value::Null);
                    let event_seq =
                        self.store.get_event_count(workflow_id).await? as i32 + 1;
                    self.store
                        .append_event(&WorkflowEvent {
                            id: None,
                            workflow_id: workflow_id.to_string(),
                            seq: event_seq,
                            event_type: "SideEffectRecorded".to_string(),
                            payload: Some(
                                serde_json::json!({
                                    "side_effect_seq": seq,
                                    "name": name,
                                    "value": value,
                                })
                                .to_string(),
                            ),
                            timestamp: timestamp_now(),
                        })
                        .await?;
                    // Side effects don't trigger anything external — the
                    // workflow needs to immediately continue so it picks
                    // up the cached value on next replay.
                    self.mark_and_emit_needs_dispatch(workflow_id).await?;
                }
                "ScheduleTimer" => {
                    let seq = cmd.get("seq").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                    let duration = cmd
                        .get("duration_secs")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
                    self.schedule_timer(workflow_id, seq, duration).await?;
                }
                "UpsertSearchAttributes" => {
                    // Merge the patch object into the workflow's stored
                    // search_attributes. Workflow code can call this from
                    // `ctx:upsert_search_attributes(...)` to surface live
                    // progress / tenant / env tags that downstream callers
                    // can filter on via the list endpoint.
                    let patch = cmd
                        .get("patch")
                        .cloned()
                        .unwrap_or(serde_json::Value::Object(Default::default()));
                    self.store
                        .upsert_search_attributes(workflow_id, &patch.to_string())
                        .await?;
                }
                "ContinueAsNew" => {
                    // Close out the current run and start a new one with the
                    // same type / namespace / queue under a fresh id. Input
                    // may be any JSON value; it's serialised and becomes the
                    // new run's `input`. Called from workflow code via
                    // `ctx:continue_as_new(input)` to reset event history
                    // when a handler would otherwise loop forever.
                    let input = cmd.get("input").map(|v| v.to_string());
                    self.continue_as_new(workflow_id, input.as_deref(), None)
                        .await?;
                }
                "RecordSnapshot" => {
                    // Persist the workflow's current query-handler state. Each
                    // snapshot is keyed by the current event seq so the latest
                    // is easy to retrieve via `get_latest_snapshot`. Runs on
                    // every worker replay, which is fine — `create_snapshot`
                    // is an insert, so each replay adds a new row reflecting
                    // the state at that point in history.
                    let state = cmd
                        .get("state")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let event_seq = self.store.get_event_count(workflow_id).await? as i32;
                    self.store
                        .create_snapshot(workflow_id, event_seq, &state.to_string())
                        .await?;
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
}
