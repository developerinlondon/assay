//! SurrealDB implementation of child-workflow listing (Task 3.14).

use std::future::Future;

use assay_core::types::WorkflowRecord;

use super::{row_to_workflow, SurrealDbStore};

impl SurrealDbStore {
    pub(crate) fn list_child_workflows_impl(
        &self,
        parent_id: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowRecord>>> + Send {
        let db = self.db.clone();
        let parent_id = parent_id.to_string();
        async move {
            let rows: Vec<serde_json::Value> = db
                .query(
                    "SELECT record::id(id) AS id, namespace, run_id, workflow_type, task_queue, \
                     status, input, result, error, parent_id, claimed_by, search_attributes, \
                     archived_at, archive_uri, needs_dispatch, dispatch_claimed_by, \
                     dispatch_last_heartbeat, created_at, updated_at, completed_at \
                     FROM workflow WHERE parent_id = $pid ORDER BY created_at ASC",
                )
                .bind(("pid", parent_id))
                .await?
                .take(0)?;
            Ok(rows.into_iter().map(row_to_workflow).collect())
        }
    }
}
