//! Activity operations and side effects.

use anyhow::Result;

use crate::ctx::{WorkflowCtx, timestamp_now};
use crate::events::WorkflowBusEvent;
use crate::store::WorkflowStore;
use crate::types::*;

impl<S: WorkflowStore> WorkflowCtx<S> {
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

        // Emit an ActivityInserted event carrying the activity row id,
        // workflow id, queue, and name — task workers subscribe and
        // dispatch matching activities (replacing the old PG trigger
        // that did `pg_notify('assay_task_<queue>', NEW.id)`).
        let ns = self
            .store
            .get_workflow(workflow_id)
            .await?
            .map(|w| w.namespace)
            .unwrap_or_else(|| "main".to_string());
        self.emit(
            &ns,
            WorkflowBusEvent::ActivityInserted {
                activity_id: id,
                workflow_id: workflow_id.to_string(),
                task_queue: task_queue.to_string(),
                name: name.to_string(),
            },
        )
        .await;

        // Transition workflow from PENDING to RUNNING on first scheduled activity
        if let Some(wf) = self.store.get_workflow(workflow_id).await?
            && wf.status == "PENDING"
        {
            self.store
                .update_workflow_status(workflow_id, WorkflowStatus::Running, None, None)
                .await?;
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

    pub async fn retry_failed_activity(
        &self,
        workflow_id: &str,
        requested_by: &str,
        reason: &str,
    ) -> Result<RetryFailedActivityResult> {
        let result = self
            .store
            .retry_failed_activity(workflow_id, requested_by, reason, timestamp_now())
            .await?;
        if let RetryFailedActivityResult::Retried(retried) = &result {
            let activity = &retried.activity;
            let namespace = self
                .store
                .get_workflow(workflow_id)
                .await?
                .map(|workflow| workflow.namespace)
                .unwrap_or_else(|| "main".to_string());
            self.emit_retry_requested(
                &namespace,
                workflow_id,
                activity.id.unwrap_or_default(),
                activity.seq,
            )
            .await;
        }
        Ok(result)
    }

    /// Mark a successfully-executed activity complete and append an
    /// `ActivityCompleted` event to the workflow event log so a replaying
    /// workflow can pick up the cached result.
    ///
    /// The row write, the history event and the dispatch arming land in one
    /// store transaction: a `COMPLETED` activity whose workflow replays it
    /// as still pending is not a reachable state. Re-calling with the same
    /// id is the repair path for a workflow task lost after the event
    /// landed — it re-arms dispatch and never rewrites the stored result.
    ///
    /// `failed=true` is preserved for legacy callers that go straight
    /// through complete with a non-retry path; new code should call
    /// [`WorkflowCtx::fail_activity`] instead so retry policy is honored.
    pub async fn complete_activity(
        &self,
        id: i64,
        result: Option<&str>,
        error: Option<&str>,
        failed: bool,
    ) -> Result<()> {
        // Read before the write: the payload needs the activity's seq and
        // name, and both are immutable once the row exists.
        let act = match self.store.get_activity(id).await? {
            Some(a) => a,
            None => return Ok(()),
        };

        let event_type = if failed {
            "ActivityFailed"
        } else {
            "ActivityCompleted"
        };
        let payload = settled_event_payload(id, act.seq, &act.name, result, error).to_string();
        let outcome = self
            .store
            .settle_activity(&ActivitySettlement {
                activity_id: id,
                workflow_id: &act.workflow_id,
                result,
                error,
                failed,
                event_type,
                payload: &payload,
                now: timestamp_now(),
            })
            .await?;
        if outcome == SettleOutcome::Repaired {
            tracing::warn!(
                activity_id = id,
                workflow_id = %act.workflow_id,
                "settled activity was missing its history event — repaired"
            );
        }
        // wake the workflow task back up.
        self.emit_needs_dispatch(&act.workflow_id).await;
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
            let backoff = act.initial_interval_secs * act.backoff_coefficient.powi(act.attempt - 1);
            let next_scheduled_at = timestamp_now() + backoff;
            self.store
                .requeue_activity_for_retry(id, act.attempt + 1, next_scheduled_at)
                .await?;
            return Ok(());
        }

        // Out of retries — mark FAILED and surface to the workflow. Row,
        // event and dispatch arming settle in one store transaction.
        let payload = failed_event_payload(id, act.seq, &act.name, error, act.attempt).to_string();
        self.store
            .settle_activity(&ActivitySettlement {
                activity_id: id,
                workflow_id: &act.workflow_id,
                result: None,
                error: Some(error),
                failed: true,
                event_type: "ActivityFailed",
                payload: &payload,
                now: timestamp_now(),
            })
            .await?;
        // Wake the workflow task — handler needs to see the failure.
        self.emit_needs_dispatch(&act.workflow_id).await;
        Ok(())
    }

    pub async fn heartbeat_activity(&self, id: i64, details: Option<&str>) -> Result<()> {
        self.store.heartbeat_activity(id, details).await
    }

    pub async fn record_side_effect(&self, workflow_id: &str, value: &str) -> Result<()> {
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

/// Payload of the `ActivityCompleted` / `ActivityFailed` event written when
/// an activity settles. Shared with the reconciler so a repaired event is
/// shaped exactly like one written inline.
pub(crate) fn settled_event_payload(
    activity_id: i64,
    activity_seq: i32,
    name: &str,
    result: Option<&str>,
    error: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "activity_id": activity_id,
        "activity_seq": activity_seq,
        "name": name,
        "result": result.and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
        "error": error,
    })
}

/// Payload of the `ActivityFailed` event written when an activity exhausts
/// its retry budget. Carries the attempt count the completion payload has
/// no reason to.
pub(crate) fn failed_event_payload(
    activity_id: i64,
    activity_seq: i32,
    name: &str,
    error: &str,
    final_attempt: i32,
) -> serde_json::Value {
    serde_json::json!({
        "activity_id": activity_id,
        "activity_seq": activity_seq,
        "name": name,
        "error": error,
        "final_attempt": final_attempt,
    })
}
