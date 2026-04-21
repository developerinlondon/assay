//! SurrealDB implementation of snapshot-related `WorkflowStore` methods (Task 3.10).

use std::future::Future;

use assay_core::types::WorkflowSnapshot;

use super::{timestamp_now, SurrealDbStore};

// ── Snapshot method impls ─────────────────────────────────────────────────────

impl SurrealDbStore {
    pub(crate) fn create_snapshot_impl(
        &self,
        workflow_id: &str,
        event_seq: i32,
        state_json: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        let db = self.db.clone();
        let workflow_id = workflow_id.to_string();
        let state_json = state_json.to_string();
        async move {
            let now = timestamp_now();
            // Composite record ID: "{workflow_id}_{event_seq}"
            let rid = format!("{workflow_id}_{event_seq}");

            // Idempotent upsert: if the record already exists, update it.
            let existing: Vec<serde_json::Value> = db
                .query("SELECT workflow_id FROM type::record('snapshot', $rid) LIMIT 1")
                .bind(("rid", rid.clone()))
                .await?
                .take(0)?;

            if existing.is_empty() {
                db.query(
                    "CREATE type::record('snapshot', $rid) CONTENT {
                        workflow_id: $wid,
                        event_seq:   $event_seq,
                        state_json:  $state_json,
                        created_at:  $created_at
                    }",
                )
                .bind(("rid", rid))
                .bind(("wid", workflow_id))
                .bind(("event_seq", event_seq))
                .bind(("state_json", state_json))
                .bind(("created_at", now))
                .await
                .map_err(|e| anyhow::anyhow!("create_snapshot: {e}"))?;
            } else {
                db.query(
                    "UPDATE type::record('snapshot', $rid) SET \
                     state_json = $state_json, created_at = $created_at",
                )
                .bind(("rid", rid))
                .bind(("state_json", state_json))
                .bind(("created_at", now))
                .await?;
            }
            Ok(())
        }
    }

    pub(crate) fn get_latest_snapshot_impl(
        &self,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowSnapshot>>> + Send {
        let db = self.db.clone();
        let workflow_id = workflow_id.to_string();
        async move {
            let rows: Vec<serde_json::Value> = db
                .query(
                    "SELECT workflow_id, event_seq, state_json, created_at
                     FROM snapshot
                     WHERE workflow_id = $wid
                     ORDER BY event_seq DESC
                     LIMIT 1",
                )
                .bind(("wid", workflow_id))
                .await?
                .take(0)?;

            Ok(rows.into_iter().next().and_then(|v| {
                Some(WorkflowSnapshot {
                    workflow_id:  v.get("workflow_id").and_then(|x| x.as_str())?.to_string(),
                    event_seq:    v.get("event_seq").and_then(|x| x.as_i64())? as i32,
                    state_json:   v.get("state_json").and_then(|x| x.as_str())?.to_string(),
                    created_at:   v.get("created_at").and_then(|x| x.as_f64()).unwrap_or(0.0),
                })
            }))
        }
    }
}
