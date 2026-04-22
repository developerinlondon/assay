//! Workflow lifecycle methods: start, get, list, cancel, terminate, complete, fail,
//! finalise_cancellation, upsert_search_attributes, notify_parent_of_child_outcome.

use std::str::FromStr;

use anyhow::Result;

use crate::ctx::{inject_engine_version, timestamp_now, WorkflowCtx};
use crate::store::WorkflowStore;
use crate::types::*;

impl<S: WorkflowStore> WorkflowCtx<S> {
    pub async fn start_workflow(
        &self,
        namespace: &str,
        workflow_type: &str,
        workflow_id: &str,
        input: Option<&str>,
        task_queue: &str,
        search_attributes: Option<&str>,
    ) -> Result<WorkflowRecord> {
        let now = timestamp_now();
        let run_id = format!("run-{workflow_id}-{}", now as u64);

        // Auto-stamp the engine version that started this run into its
        // search attributes. Makes post-mortem triage concrete: "this
        // run was v0.11.9, that's why it was stuck in main instead of
        // deployments" is the kind of question that's otherwise guesswork
        // once multiple engine versions have coexisted in a deployment.
        // The operator's own attributes take precedence if they also
        // supplied `assay_engine_version` — we don't overwrite, just
        // backfill on the "not set" case.
        let stamped_attrs = inject_engine_version(search_attributes);

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
            search_attributes: stamped_attrs,
            archived_at: None,
            archive_uri: None,
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

        // Notify SSE subscribers so the dashboard row appears live.
        self.broadcast("workflow_started", workflow_id, namespace);

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
        search_attrs_filter: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<WorkflowRecord>> {
        self.store
            .list_workflows(
                namespace,
                status,
                workflow_type,
                search_attrs_filter,
                limit,
                offset,
            )
            .await
    }

    pub async fn upsert_search_attributes(
        &self,
        workflow_id: &str,
        patch_json: &str,
    ) -> Result<()> {
        self.store
            .upsert_search_attributes(workflow_id, patch_json)
            .await
    }

    pub async fn cancel_workflow(&self, id: &str, reason: Option<&str>) -> Result<bool> {
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

                // Operator-supplied reason rides along on the event
                // payload so audit queries (and the dashboard's events
                // tab) can show why a cancel happened. Symmetric with
                // terminate, which has always taken a reason.
                let payload = reason.map(|r| {
                    serde_json::json!({ "reason": r }).to_string()
                });

                let seq = self.store.get_event_count(id).await? as i32 + 1;
                self.store
                    .append_event(&WorkflowEvent {
                        id: None,
                        workflow_id: id.to_string(),
                        seq,
                        event_type: "WorkflowCancelRequested".to_string(),
                        payload,
                        timestamp: timestamp_now(),
                    })
                    .await?;

                self.store.mark_workflow_dispatchable(id).await?;

                // Propagate cancellation to all child workflows. Children
                // inherit the reason so the audit trail explains the
                // whole cascade in one place.
                let children = self.store.list_child_workflows(id).await?;
                for child in children {
                    Box::pin(self.cancel_workflow(&child.id, reason)).await?;
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

                // Live-refresh the dashboard — no more F5 after
                // Terminate.
                self.broadcast("workflow_terminated", id, &wf.namespace);

                Ok(true)
            }
        }
    }

    /// Finalise a cancellation: flips status to CANCELLED and appends the
    /// terminal WorkflowCancelled event. Called by the CancelWorkflow
    /// command handler (worker acknowledged cancel) and by cancel_workflow
    /// directly when the workflow has no worker yet.
    pub async fn finalise_cancellation(&self, workflow_id: &str) -> Result<()> {
        // Avoid double-finalising
        if let Some(wf) = self.store.get_workflow(workflow_id).await?
            && wf.status == "CANCELLED"
        {
            return Ok(());
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
        let ns = self
            .store
            .get_workflow(workflow_id)
            .await?
            .map(|w| w.namespace)
            .unwrap_or_default();
        self.broadcast("workflow_cancelled", workflow_id, &ns);
        Ok(())
    }

    /// Mark a workflow COMPLETED with a result + append WorkflowCompleted event.
    /// If the workflow has a parent, also notifies the parent with a
    /// ChildWorkflowCompleted event and marks it dispatchable so it can
    /// replay past `ctx:start_child_workflow` and pick up the child's result.
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
        self.notify_parent_of_child_outcome(
            workflow_id,
            "ChildWorkflowCompleted",
            serde_json::json!({
                "child_workflow_id": workflow_id,
                "result": result.and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
            }),
        )
        .await?;
        let ns = self
            .store
            .get_workflow(workflow_id)
            .await?
            .map(|w| w.namespace)
            .unwrap_or_default();
        self.broadcast("workflow_completed", workflow_id, &ns);
        Ok(())
    }

    /// Mark a workflow FAILED with an error + append WorkflowFailed event.
    /// Notifies the parent if any (ChildWorkflowFailed).
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
        self.notify_parent_of_child_outcome(
            workflow_id,
            "ChildWorkflowFailed",
            serde_json::json!({
                "child_workflow_id": workflow_id,
                "error": error,
            }),
        )
        .await?;
        let ns = self
            .store
            .get_workflow(workflow_id)
            .await?
            .map(|w| w.namespace)
            .unwrap_or_default();
        self.broadcast("workflow_failed", workflow_id, &ns);
        Ok(())
    }

    /// Append a parent-side event when a child reaches a terminal state and
    /// re-dispatch the parent so it can replay past its `start_child_workflow`
    /// call. No-op for top-level workflows (no parent_id).
    async fn notify_parent_of_child_outcome(
        &self,
        child_workflow_id: &str,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<()> {
        let Some(child) = self.store.get_workflow(child_workflow_id).await? else {
            return Ok(());
        };
        let Some(parent_id) = child.parent_id else {
            return Ok(());
        };
        let event_seq = self.store.get_event_count(&parent_id).await? as i32 + 1;
        self.store
            .append_event(&WorkflowEvent {
                id: None,
                workflow_id: parent_id.clone(),
                seq: event_seq,
                event_type: event_type.to_string(),
                payload: Some(payload.to_string()),
                timestamp: timestamp_now(),
            })
            .await?;
        self.store.mark_workflow_dispatchable(&parent_id).await?;
        Ok(())
    }
}
