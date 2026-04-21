//! SurrealDB implementation of timer-related `WorkflowStore` methods.

use std::future::Future;

use assay_core::types::WorkflowTimer;

use super::SurrealDbStore;

// ── Helper ────────────────────────────────────────────────────────────────────

pub(super) fn row_to_timer(v: serde_json::Value) -> WorkflowTimer {
    WorkflowTimer {
        id: v.get("id_num").and_then(|x| x.as_i64()),
        workflow_id: v.get("workflow_id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        seq: v.get("seq").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
        fire_at: v.get("fire_at").and_then(|x| x.as_f64()).unwrap_or(0.0),
        fired: v.get("fired").and_then(|x| x.as_bool()).unwrap_or(false),
    }
}

async fn next_timer_id(db: &surrealdb::Surreal<surrealdb::engine::remote::ws::Client>) -> anyhow::Result<i64> {
    let rows: Vec<serde_json::Value> = db
        .query("UPDATE _seq SET val = val + 1 WHERE name = $name RETURN val")
        .bind(("name", "timer".to_string()))
        .await?
        .take(0)?;
    rows.first()
        .and_then(|v| v.get("val"))
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("next_timer_id: counter row missing"))
}

// ── Timer method impls ────────────────────────────────────────────────────────

impl SurrealDbStore {
    pub(crate) fn create_timer_impl(
        &self,
        timer: &WorkflowTimer,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send {
        let db = self.db.clone();
        let t = timer.clone();
        async move {
            // Idempotent: if (workflow_id, seq) already exists, return existing id_num.
            let existing: Vec<serde_json::Value> = db
                .query(
                    "SELECT id_num FROM timer WHERE workflow_id = $wid AND seq = $seq LIMIT 1",
                )
                .bind(("wid", t.workflow_id.clone()))
                .bind(("seq", t.seq))
                .await?
                .take(0)?;

            if let Some(row) = existing.into_iter().next() {
                let existing_id = row.get("id_num").and_then(|v| v.as_i64()).unwrap_or(0);
                return Ok(existing_id);
            }

            let id_num = next_timer_id(&db).await?;
            let record_id = format!("{}_{}", t.workflow_id, t.seq);

            db.query(
                "CREATE type::record('timer', $rid) CONTENT {
                    id_num:     $id_num,
                    workflow_id: $workflow_id,
                    seq:        $seq,
                    fire_at:    $fire_at,
                    fired:      false
                }",
            )
            .bind(("rid", record_id))
            .bind(("id_num", id_num))
            .bind(("workflow_id", t.workflow_id.clone()))
            .bind(("seq", t.seq))
            .bind(("fire_at", t.fire_at))
            .await
            .map_err(|e| anyhow::anyhow!("create_timer({}:{}): {e}", t.workflow_id, t.seq))?;

            Ok(id_num)
        }
    }

    pub(crate) fn get_timer_by_workflow_seq_impl(
        &self,
        workflow_id: &str,
        seq: i32,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowTimer>>> + Send {
        let db = self.db.clone();
        let workflow_id = workflow_id.to_string();
        async move {
            let rows: Vec<serde_json::Value> = db
                .query(
                    "SELECT id_num, workflow_id, seq, fire_at, fired
                     FROM timer WHERE workflow_id = $wid AND seq = $seq LIMIT 1",
                )
                .bind(("wid", workflow_id))
                .bind(("seq", seq))
                .await?
                .take(0)?;
            Ok(rows.into_iter().next().map(row_to_timer))
        }
    }

    pub(crate) fn fire_due_timers_impl(
        &self,
        now: f64,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowTimer>>> + Send {
        let db = self.db.clone();
        async move {
            // Find due timers first, then atomically flip fired=true.
            // SurrealDB UPDATE ... WHERE ... RETURN gives back the updated rows.
            let updated: Vec<serde_json::Value> = db
                .query(
                    "UPDATE timer SET fired = true
                     WHERE fired = false AND fire_at <= $now
                     RETURN id_num, workflow_id, seq, fire_at, fired",
                )
                .bind(("now", now))
                .await?
                .take(0)?;
            Ok(updated.into_iter().map(row_to_timer).collect())
        }
    }

    pub(crate) fn cancel_pending_timers_impl(
        &self,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<u64>> + Send {
        let db = self.db.clone();
        let workflow_id = workflow_id.to_string();
        async move {
            let updated: Vec<serde_json::Value> = db
                .query(
                    "UPDATE timer SET fired = true
                     WHERE workflow_id = $wid AND fired = false
                     RETURN id_num",
                )
                .bind(("wid", workflow_id))
                .await?
                .take(0)?;
            Ok(updated.len() as u64)
        }
    }
}
