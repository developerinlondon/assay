//! Child workflow operations and continue-as-new.

use std::str::FromStr;

use anyhow::Result;

use crate::ctx::{WorkflowCtx, strip_continued_suffix, timestamp_now};
use crate::store::WorkflowStore;
use crate::types::*;

impl<S: WorkflowStore> WorkflowCtx<S> {
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
            search_attributes: None,
            archived_at: None,
            archive_uri: None,
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

    pub async fn list_child_workflows(&self, parent_id: &str) -> Result<Vec<WorkflowRecord>> {
        self.store.list_child_workflows(parent_id).await
    }

    pub async fn continue_as_new(
        &self,
        workflow_id: &str,
        input: Option<&str>,
        explicit_new_id: Option<&str>,
    ) -> Result<WorkflowRecord> {
        let old_wf = self
            .store
            .get_workflow(workflow_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("workflow not found: {workflow_id}"))?;

        let old_status =
            WorkflowStatus::from_str(&old_wf.status).map_err(|e| anyhow::anyhow!(e))?;

        // Only close the old run when it's still in-flight. A workflow
        // that's already CANCELLED / FAILED / COMPLETED / TERMINATED
        // stays that way — overwriting the terminal status would lose
        // audit history (e.g. a cancelled run flipping to COMPLETED
        // when the operator hits "Start a fresh run" on the row).
        if !old_status.is_terminal() {
            self.store
                .update_workflow_status(workflow_id, WorkflowStatus::Completed, None, None)
                .await?;
        }

        // Start a new run with the same type, namespace, and queue.
        // Naming:
        //   1. Caller-provided explicit id (e.g. dashboard combobox) —
        //      honoured verbatim.
        //   2. Otherwise derive: strip any existing `-continued-<digits>`
        //      suffix from the source so sequential continues don't
        //      stack (`demo-1` → `demo-1-continued-1` → `demo-1-continued-2`
        //      instead of the compounding `demo-1-continued-1-continued-2`).
        let new_id = match explicit_new_id {
            Some(id) => id.to_string(),
            None => {
                let base = strip_continued_suffix(workflow_id);
                format!("{base}-continued-{}", timestamp_now() as u64)
            }
        };
        self.start_workflow(
            &old_wf.namespace,
            &old_wf.workflow_type,
            &new_id,
            input,
            &old_wf.task_queue,
            old_wf.search_attributes.as_deref(),
        )
        .await
    }
}
