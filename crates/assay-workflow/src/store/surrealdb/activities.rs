//! SurrealDB implementation of activity-related `WorkflowStore` methods.

use std::future::Future;

use assay_core::types::WorkflowActivity;

use super::SurrealDbStore;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn timestamp_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// Convert a `serde_json::Value` row (from a SurrealDB activity SELECT) into a
/// `WorkflowActivity`.  All fields are optional in JSON — we use safe defaults
/// so the conversion never fails.
pub(super) fn row_to_activity(v: serde_json::Value) -> WorkflowActivity {
    WorkflowActivity {
        id: v.get("id_num").and_then(|x| x.as_i64()),
        workflow_id: v.get("workflow_id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        seq: v.get("seq").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
        name: v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        task_queue: v.get("task_queue").and_then(|x| x.as_str()).unwrap_or("main").to_string(),
        input: v.get("input").and_then(|x| if x.is_null() { None } else { x.as_str().map(|s| s.to_string()) }),
        status: v.get("status").and_then(|x| x.as_str()).unwrap_or("PENDING").to_string(),
        result: v.get("result").and_then(|x| if x.is_null() { None } else { x.as_str().map(|s| s.to_string()) }),
        error: v.get("error").and_then(|x| if x.is_null() { None } else { x.as_str().map(|s| s.to_string()) }),
        attempt: v.get("attempt").and_then(|x| x.as_i64()).unwrap_or(1) as i32,
        max_attempts: v.get("max_attempts").and_then(|x| x.as_i64()).unwrap_or(3) as i32,
        initial_interval_secs: v.get("initial_interval_secs").and_then(|x| x.as_f64()).unwrap_or(1.0),
        backoff_coefficient: v.get("backoff_coefficient").and_then(|x| x.as_f64()).unwrap_or(2.0),
        start_to_close_secs: v.get("start_to_close_secs").and_then(|x| x.as_f64()).unwrap_or(300.0),
        heartbeat_timeout_secs: v.get("heartbeat_timeout_secs").and_then(|x| if x.is_null() { None } else { x.as_f64() }),
        claimed_by: v.get("claimed_by").and_then(|x| if x.is_null() { None } else { x.as_str().map(|s| s.to_string()) }),
        scheduled_at: v.get("scheduled_at").and_then(|x| x.as_f64()).unwrap_or(0.0),
        started_at: v.get("started_at").and_then(|x| if x.is_null() { None } else { x.as_f64() }),
        completed_at: v.get("completed_at").and_then(|x| if x.is_null() { None } else { x.as_f64() }),
        last_heartbeat: v.get("last_heartbeat").and_then(|x| if x.is_null() { None } else { x.as_f64() }),
    }
}

/// Allocate the next integer ID for the given sequence name.
/// Uses a read-then-update on the `_seq` table.  Not perfectly atomic under
/// concurrent inserts, but creates unique IDs in the absence of hardware-level
/// races because SurrealDB serialises record-level updates.
async fn next_seq(db: &surrealdb::Surreal<surrealdb::engine::remote::ws::Client>, seq_name: &str) -> anyhow::Result<i64> {
    // Increment val and return the new value.
    let rows: Vec<serde_json::Value> = db
        .query("UPDATE _seq SET val = val + 1 WHERE name = $name RETURN val")
        .bind(("name", seq_name.to_string()))
        .await?
        .take(0)?;
    let id = rows
        .first()
        .and_then(|v| v.get("val"))
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("next_seq({seq_name}): counter row missing"))?;
    Ok(id)
}

// ── Activity method impls ─────────────────────────────────────────────────────

impl SurrealDbStore {
    pub(crate) fn create_activity_impl(
        &self,
        activity: &WorkflowActivity,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send {
        let db = self.db.clone();
        let act = activity.clone();
        async move {
            // Idempotent create-by-(workflow_id, seq): check first, create only if absent.
            let existing: Vec<serde_json::Value> = db
                .query(
                    "SELECT id_num FROM activity WHERE workflow_id = $wid AND seq = $seq LIMIT 1",
                )
                .bind(("wid", act.workflow_id.clone()))
                .bind(("seq", act.seq))
                .await?
                .take(0)?;

            if let Some(row) = existing.into_iter().next() {
                let existing_id = row.get("id_num").and_then(|v| v.as_i64()).unwrap_or(0);
                return Ok(existing_id);
            }

            let id_num = next_seq(&db, "activity").await?;
            // Composite record key: "{workflow_id}_{seq}"
            let record_id = format!("{}_{}", act.workflow_id, act.seq);

            db.query(
                "CREATE type::record('activity', $rid) CONTENT {
                    id_num:                $id_num,
                    workflow_id:           $workflow_id,
                    seq:                   $seq,
                    name:                  $name,
                    task_queue:            $task_queue,
                    input:                 $input,
                    status:                $status,
                    result:                NONE,
                    error:                 NONE,
                    attempt:               $attempt,
                    max_attempts:          $max_attempts,
                    initial_interval_secs: $initial_interval_secs,
                    backoff_coefficient:   $backoff_coefficient,
                    start_to_close_secs:   $start_to_close_secs,
                    heartbeat_timeout_secs:$heartbeat_timeout_secs,
                    claimed_by:            NONE,
                    scheduled_at:          $scheduled_at,
                    started_at:            NONE,
                    completed_at:          NONE,
                    last_heartbeat:        NONE
                }",
            )
            .bind(("rid", record_id))
            .bind(("id_num", id_num))
            .bind(("workflow_id", act.workflow_id.clone()))
            .bind(("seq", act.seq))
            .bind(("name", act.name.clone()))
            .bind(("task_queue", act.task_queue.clone()))
            .bind(("input", act.input.clone()))
            .bind(("status", act.status.clone()))
            .bind(("attempt", act.attempt))
            .bind(("max_attempts", act.max_attempts))
            .bind(("initial_interval_secs", act.initial_interval_secs))
            .bind(("backoff_coefficient", act.backoff_coefficient))
            .bind(("start_to_close_secs", act.start_to_close_secs))
            .bind(("heartbeat_timeout_secs", act.heartbeat_timeout_secs))
            .bind(("scheduled_at", act.scheduled_at))
            .await
            .map_err(|e| anyhow::anyhow!("create_activity({}:{}): {e}", act.workflow_id, act.seq))?;

            Ok(id_num)
        }
    }

    pub(crate) fn get_activity_impl(
        &self,
        id: i64,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowActivity>>> + Send {
        let db = self.db.clone();
        async move {
            let rows: Vec<serde_json::Value> = db
                .query(
                    "SELECT id_num, workflow_id, seq, name, task_queue, input, status, result, error,
                            attempt, max_attempts, initial_interval_secs, backoff_coefficient,
                            start_to_close_secs, heartbeat_timeout_secs, claimed_by,
                            scheduled_at, started_at, completed_at, last_heartbeat
                     FROM activity WHERE id_num = $id LIMIT 1",
                )
                .bind(("id", id))
                .await?
                .take(0)?;
            Ok(rows.into_iter().next().map(row_to_activity))
        }
    }

    pub(crate) fn get_activity_by_workflow_seq_impl(
        &self,
        workflow_id: &str,
        seq: i32,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowActivity>>> + Send {
        let db = self.db.clone();
        let workflow_id = workflow_id.to_string();
        async move {
            let rows: Vec<serde_json::Value> = db
                .query(
                    "SELECT id_num, workflow_id, seq, name, task_queue, input, status, result, error,
                            attempt, max_attempts, initial_interval_secs, backoff_coefficient,
                            start_to_close_secs, heartbeat_timeout_secs, claimed_by,
                            scheduled_at, started_at, completed_at, last_heartbeat
                     FROM activity WHERE workflow_id = $wid AND seq = $seq LIMIT 1",
                )
                .bind(("wid", workflow_id))
                .bind(("seq", seq))
                .await?
                .take(0)?;
            Ok(rows.into_iter().next().map(row_to_activity))
        }
    }

    pub(crate) fn claim_activity_impl(
        &self,
        task_queue: &str,
        worker_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowActivity>>> + Send {
        let db = self.db.clone();
        let task_queue = task_queue.to_string();
        let worker_id = worker_id.to_string();
        async move {
            let now = timestamp_now();
            // Step 1: find oldest PENDING activity on this queue.
            // SurrealDB requires ORDER BY fields to appear in the SELECT list.
            let candidates: Vec<serde_json::Value> = db
                .query(
                    "SELECT id_num, workflow_id, seq, scheduled_at FROM activity
                     WHERE task_queue = $tq AND status = 'PENDING'
                     ORDER BY scheduled_at ASC
                     LIMIT 1",
                )
                .bind(("tq", task_queue))
                .await?
                .take(0)?;

            let candidate = match candidates.into_iter().next() {
                Some(c) => c,
                None => return Ok(None),
            };

            let id_num = match candidate.get("id_num").and_then(|v| v.as_i64()) {
                Some(id) => id,
                None => return Ok(None),
            };

            // Step 2: atomically claim using WHERE status = 'PENDING' to detect races.
            let updated: Vec<serde_json::Value> = db
                .query(
                    "UPDATE activity
                     SET status = 'RUNNING', claimed_by = $worker, started_at = $now
                     WHERE id_num = $id AND status = 'PENDING'
                     RETURN id_num, workflow_id, seq, name, task_queue, input, status, result, error,
                            attempt, max_attempts, initial_interval_secs, backoff_coefficient,
                            start_to_close_secs, heartbeat_timeout_secs, claimed_by,
                            scheduled_at, started_at, completed_at, last_heartbeat",
                )
                .bind(("id", id_num))
                .bind(("worker", worker_id))
                .bind(("now", now))
                .await?
                .take(0)?;

            Ok(updated.into_iter().next().map(row_to_activity))
        }
    }

    pub(crate) fn requeue_activity_for_retry_impl(
        &self,
        id: i64,
        next_attempt: i32,
        next_scheduled_at: f64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        let db = self.db.clone();
        async move {
            db.query(
                "UPDATE activity
                 SET status = 'PENDING', attempt = $attempt, scheduled_at = $scheduled_at,
                     claimed_by = NONE, started_at = NONE, last_heartbeat = NONE, error = NONE
                 WHERE id_num = $id",
            )
            .bind(("id", id))
            .bind(("attempt", next_attempt))
            .bind(("scheduled_at", next_scheduled_at))
            .await?;
            Ok(())
        }
    }

    pub(crate) fn complete_activity_impl(
        &self,
        id: i64,
        result: Option<&str>,
        error: Option<&str>,
        failed: bool,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        let db = self.db.clone();
        let status = if failed { "FAILED" } else { "COMPLETED" };
        let result = result.map(|s| s.to_string());
        let error = error.map(|s| s.to_string());
        async move {
            let now = timestamp_now();
            db.query(
                "UPDATE activity
                 SET status = $status, result = $result, error = $error, completed_at = $now
                 WHERE id_num = $id",
            )
            .bind(("id", id))
            .bind(("status", status))
            .bind(("result", result))
            .bind(("error", error))
            .bind(("now", now))
            .await?;
            Ok(())
        }
    }

    pub(crate) fn heartbeat_activity_impl(
        &self,
        id: i64,
        _details: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        let db = self.db.clone();
        async move {
            let now = timestamp_now();
            db.query(
                "UPDATE activity SET last_heartbeat = $now WHERE id_num = $id",
            )
            .bind(("id", id))
            .bind(("now", now))
            .await?;
            Ok(())
        }
    }

    pub(crate) fn get_timed_out_activities_impl(
        &self,
        now: f64,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowActivity>>> + Send {
        let db = self.db.clone();
        async move {
            let rows: Vec<serde_json::Value> = db
                .query(
                    "SELECT id_num, workflow_id, seq, name, task_queue, input, status, result, error,
                            attempt, max_attempts, initial_interval_secs, backoff_coefficient,
                            start_to_close_secs, heartbeat_timeout_secs, claimed_by,
                            scheduled_at, started_at, completed_at, last_heartbeat
                     FROM activity
                     WHERE status = 'RUNNING'
                       AND heartbeat_timeout_secs != NONE
                       AND last_heartbeat != NONE
                       AND ($now - last_heartbeat) > heartbeat_timeout_secs",
                )
                .bind(("now", now))
                .await?
                .take(0)?;
            Ok(rows.into_iter().map(row_to_activity).collect())
        }
    }

    pub(crate) fn cancel_pending_activities_impl(
        &self,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<u64>> + Send {
        let db = self.db.clone();
        let workflow_id = workflow_id.to_string();
        async move {
            let now = timestamp_now();
            // Count before update — SurrealDB UPDATE returns the updated rows.
            let updated: Vec<serde_json::Value> = db
                .query(
                    "UPDATE activity
                     SET status = 'CANCELLED', completed_at = $now
                     WHERE workflow_id = $wid AND status = 'PENDING'
                     RETURN id_num",
                )
                .bind(("wid", workflow_id))
                .bind(("now", now))
                .await?
                .take(0)?;
            Ok(updated.len() as u64)
        }
    }
}
