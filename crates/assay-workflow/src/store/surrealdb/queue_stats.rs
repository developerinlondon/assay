//! SurrealDB implementation of queue-stats `WorkflowStore` methods (Task 3.14).
//!
//! PG uses a JOIN across activity + worker tables grouped by task_queue.
//! SurrealDB equivalent: aggregate activity rows per task_queue, then look up
//! worker counts per queue separately and merge.

use std::future::Future;

use assay_core::QueueStats;

use super::SurrealDbStore;

impl SurrealDbStore {
    pub(crate) fn get_queue_stats_impl(
        &self,
        namespace: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<QueueStats>>> + Send {
        let db = self.db.clone();
        let namespace = namespace.to_string();
        async move {
            // Step 1: collect all workflow IDs in the namespace.
            // We then aggregate activity rows by task_queue for those workflows.
            //
            // SurrealDB doesn't support subqueries in WHERE or JOIN, so we use
            // two queries and merge in Rust.

            // Get activities grouped by task_queue where the parent workflow is
            // in this namespace.  SurrealDB GROUP BY on SCHEMAFULL tables
            // requires all selected fields to be in the GROUP clause, so we use
            // count() with GROUP to aggregate.
            //
            // Strategy:
            //   a) Get distinct task_queues from activity rows whose workflow_id
            //      matches a workflow in this namespace.
            //   b) For each queue, count PENDING and RUNNING activities.
            //   c) Count workers registered to each queue in this namespace.

            // Fetch all workflow IDs in the namespace.
            let wf_rows: Vec<serde_json::Value> = db
                .query("SELECT record::id(id) AS wid FROM workflow WHERE namespace = $ns")
                .bind(("ns", namespace.clone()))
                .await?
                .take(0)?;

            let wf_ids: Vec<String> = wf_rows
                .into_iter()
                .filter_map(|v| v.get("wid").and_then(|x| x.as_str()).map(|s| s.to_string()))
                .collect();

            if wf_ids.is_empty() {
                return Ok(vec![]);
            }

            // Get all activity rows for those workflows (status + task_queue).
            // We pull status + task_queue and aggregate in Rust to avoid
            // SurrealDB GROUP BY limitations with computed fields.
            let act_rows: Vec<serde_json::Value> = db
                .query("SELECT task_queue, status FROM activity WHERE workflow_id INSIDE $wids")
                .bind(("wids", wf_ids))
                .await?
                .take(0)?;

            // Aggregate in Rust: map task_queue -> (pending, running).
            use std::collections::HashMap;
            let mut queue_map: HashMap<String, (i64, i64)> = HashMap::new();
            for row in &act_rows {
                let queue = row
                    .get("task_queue")
                    .and_then(|x| x.as_str())
                    .unwrap_or("main")
                    .to_string();
                let status = row
                    .get("status")
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                let entry = queue_map.entry(queue).or_insert((0, 0));
                match status {
                    "PENDING" => entry.0 += 1,
                    "RUNNING" => entry.1 += 1,
                    _ => {}
                }
            }

            if queue_map.is_empty() {
                return Ok(vec![]);
            }

            // For each queue, count workers in this namespace whose task_queue matches.
            let mut stats: Vec<QueueStats> = Vec::new();
            for (queue, (pending, running)) in queue_map {
                let worker_rows: Vec<serde_json::Value> = db
                    .query(
                        "SELECT count() AS c FROM worker WHERE namespace = $ns AND task_queue = $tq GROUP ALL",
                    )
                    .bind(("ns", namespace.clone()))
                    .bind(("tq", queue.clone()))
                    .await?
                    .take(0)?;
                let workers = worker_rows
                    .first()
                    .and_then(|v| v.get("c"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                stats.push(QueueStats {
                    queue,
                    pending_activities: pending,
                    running_activities: running,
                    workers,
                });
            }

            // Stable sort for deterministic test output.
            stats.sort_by(|a, b| a.queue.cmp(&b.queue));
            Ok(stats)
        }
    }
}
