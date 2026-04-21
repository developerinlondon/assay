//! SurrealDB implementation of worker-related `WorkflowStore` methods (Task 3.12).

use std::future::Future;

use assay_core::types::WorkflowWorker;

use super::SurrealDbStore;

// ── Helper ────────────────────────────────────────────────────────────────────

fn strip_record_prefix(s: &str) -> &str {
    // SurrealDB v3 may return record IDs as "table:`raw_id`" or "table:raw_id".
    // Strip everything up to and including the first ':' and any surrounding backticks.
    if let Some(pos) = s.find(':') {
        let after = s[pos + 1..].trim_matches('`');
        return after;
    }
    s
}

fn row_to_worker(v: serde_json::Value) -> Option<WorkflowWorker> {
    let raw_id = v.get("worker_id").and_then(|x| x.as_str())?;
    let id = strip_record_prefix(raw_id).to_string();
    Some(WorkflowWorker {
        id,
        namespace:                   v.get("namespace").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        identity:                    v.get("identity").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        task_queue:                  v.get("task_queue").and_then(|x| x.as_str()).unwrap_or("main").to_string(),
        workflows:                   v.get("workflows").and_then(|x| if x.is_null() { None } else { x.as_str().map(|s| s.to_string()) }),
        activities:                  v.get("activities").and_then(|x| if x.is_null() { None } else { x.as_str().map(|s| s.to_string()) }),
        max_concurrent_workflows:    v.get("max_concurrent_workflows").and_then(|x| x.as_i64()).unwrap_or(10) as i32,
        max_concurrent_activities:   v.get("max_concurrent_activities").and_then(|x| x.as_i64()).unwrap_or(10) as i32,
        active_tasks:                v.get("active_tasks").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
        last_heartbeat:              v.get("last_heartbeat").and_then(|x| x.as_f64()).unwrap_or(0.0),
        registered_at:               v.get("registered_at").and_then(|x| x.as_f64()).unwrap_or(0.0),
    })
}

const SELECT_FIELDS: &str =
    "id AS worker_id, namespace, identity, task_queue, workflows, activities, \
     max_concurrent_workflows, max_concurrent_activities, active_tasks, \
     last_heartbeat, registered_at";

// ── Worker method impls ───────────────────────────────────────────────────────

impl SurrealDbStore {
    pub(crate) fn register_worker_impl(
        &self,
        worker: &WorkflowWorker,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        let db = self.db.clone();
        let w = worker.clone();
        async move {
            // Idempotent upsert: if record exists, update heartbeat + identity.
            let existing: Vec<serde_json::Value> = db
                .query("SELECT id FROM type::record('worker', $wid) LIMIT 1")
                .bind(("wid", w.id.clone()))
                .await?
                .take(0)?;

            if existing.is_empty() {
                db.query(
                    "CREATE type::record('worker', $wid) CONTENT {
                        id:                        $wid,
                        namespace:                 $ns,
                        identity:                  $identity,
                        task_queue:                $task_queue,
                        task_queues:               [$task_queue],
                        workflows:                 $workflows,
                        activities:                $activities,
                        max_concurrent_workflows:  $max_concurrent_workflows,
                        max_concurrent_activities: $max_concurrent_activities,
                        active_tasks:              $active_tasks,
                        last_heartbeat:            $last_heartbeat,
                        heartbeat_at:              $last_heartbeat,
                        registered_at:             $registered_at
                    }",
                )
                .bind(("wid", w.id.clone()))
                .bind(("ns", w.namespace.clone()))
                .bind(("identity", w.identity.clone()))
                .bind(("task_queue", w.task_queue.clone()))
                .bind(("workflows", w.workflows.clone()))
                .bind(("activities", w.activities.clone()))
                .bind(("max_concurrent_workflows", w.max_concurrent_workflows))
                .bind(("max_concurrent_activities", w.max_concurrent_activities))
                .bind(("active_tasks", w.active_tasks))
                .bind(("last_heartbeat", w.last_heartbeat))
                .bind(("registered_at", w.registered_at))
                .await
                .map_err(|e| anyhow::anyhow!("register_worker({}): {e}", w.id))?;
            } else {
                // ON CONFLICT: update heartbeat + identity (mirror PG ON CONFLICT DO UPDATE).
                db.query(
                    "UPDATE type::record('worker', $wid) SET \
                     last_heartbeat = $last_heartbeat, \
                     identity = $identity",
                )
                .bind(("wid", w.id.clone()))
                .bind(("last_heartbeat", w.last_heartbeat))
                .bind(("identity", w.identity.clone()))
                .await?;
            }
            Ok(())
        }
    }

    pub(crate) fn heartbeat_worker_impl(
        &self,
        id: &str,
        now: f64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        let db = self.db.clone();
        let id = id.to_string();
        async move {
            db.query("UPDATE type::record('worker', $wid) SET last_heartbeat = $now")
                .bind(("wid", id))
                .bind(("now", now))
                .await?;
            Ok(())
        }
    }

    pub(crate) fn list_workers_impl(
        &self,
        namespace: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowWorker>>> + Send {
        let db = self.db.clone();
        let namespace = namespace.to_string();
        async move {
            let sql = format!(
                "SELECT {SELECT_FIELDS} FROM worker WHERE namespace = $ns ORDER BY registered_at ASC"
            );
            let rows: Vec<serde_json::Value> = db
                .query(&sql)
                .bind(("ns", namespace))
                .await?
                .take(0)?;
            Ok(rows.into_iter().filter_map(row_to_worker).collect())
        }
    }

    pub(crate) fn remove_dead_workers_impl(
        &self,
        cutoff: f64,
    ) -> impl Future<Output = anyhow::Result<Vec<String>>> + Send {
        let db = self.db.clone();
        async move {
            // Use record::id(id) to get the raw string ID (not the full record-ID object).
            let rows: Vec<serde_json::Value> = db
                .query("SELECT record::id(id) AS raw_id FROM worker WHERE last_heartbeat < $cutoff")
                .bind(("cutoff", cutoff))
                .await?
                .take(0)?;

            let ids: Vec<String> = rows
                .into_iter()
                .filter_map(|v| {
                    v.get("raw_id")
                        .and_then(|x| x.as_str())
                        .map(|s| strip_record_prefix(s).to_string())
                })
                .collect();

            if !ids.is_empty() {
                db.query("DELETE worker WHERE last_heartbeat < $cutoff")
                    .bind(("cutoff", cutoff))
                    .await?;
            }
            Ok(ids)
        }
    }
}
