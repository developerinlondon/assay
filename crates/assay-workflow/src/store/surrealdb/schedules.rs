//! SurrealDB implementation of schedule-related `WorkflowStore` methods (Task 3.9).

use std::future::Future;

use assay_core::types::{SchedulePatch, WorkflowSchedule};

use super::SurrealDbStore;

// ── Helper ────────────────────────────────────────────────────────────────────

pub(super) fn row_to_schedule(v: serde_json::Value) -> Option<WorkflowSchedule> {
    Some(WorkflowSchedule {
        namespace:        v.get("namespace").and_then(|x| x.as_str())?.to_string(),
        name:             v.get("sched_name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        workflow_type:    v.get("workflow_type").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        cron_expr:        v.get("cron_expr").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        timezone:         v.get("timezone").and_then(|x| x.as_str()).unwrap_or("UTC").to_string(),
        input:            v.get("input").and_then(|x| if x.is_null() { None } else { x.as_str().map(|s| s.to_string()) }),
        task_queue:       v.get("task_queue").and_then(|x| x.as_str()).unwrap_or("main").to_string(),
        overlap_policy:   v.get("overlap_policy").and_then(|x| x.as_str()).unwrap_or("skip").to_string(),
        paused:           v.get("paused").and_then(|x| x.as_bool()).unwrap_or(false),
        last_run_at:      v.get("last_run_at").and_then(|x| if x.is_null() { None } else { x.as_f64() }),
        next_run_at:      v.get("next_run_at").and_then(|x| if x.is_null() { None } else { x.as_f64() }),
        last_workflow_id: v.get("last_workflow_id").and_then(|x| if x.is_null() { None } else { x.as_str().map(|s| s.to_string()) }),
        created_at:       v.get("created_at").and_then(|x| x.as_f64()).unwrap_or(0.0),
    })
}

// ── Record-ID helper ──────────────────────────────────────────────────────────

/// Build the composite record key used for schedule records: `{namespace}_{name}`.
/// We replace characters that are special in SurrealDB record IDs with underscores.
fn schedule_rid(namespace: &str, name: &str) -> String {
    // SurrealDB record IDs with `type::record('schedule', $rid)` allow any string
    // as the ID component; the angle-bracket quoting handles special chars.
    format!("{namespace}_{name}")
}

const SELECT_FIELDS: &str =
    "namespace, name AS sched_name, workflow_type, cron_expr, timezone, input, \
     task_queue, overlap_policy, paused, last_run_at, next_run_at, last_workflow_id, created_at";

// ── Schedule method impls ─────────────────────────────────────────────────────

impl SurrealDbStore {
    pub(crate) fn create_schedule_impl(
        &self,
        schedule: &WorkflowSchedule,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        let db = self.db.clone();
        let s = schedule.clone();
        async move {
            let rid = schedule_rid(&s.namespace, &s.name);
            // Idempotent: SELECT first; if exists, skip.
            let existing: Vec<serde_json::Value> = db
                .query("SELECT namespace FROM type::record('schedule', $rid) LIMIT 1")
                .bind(("rid", rid.clone()))
                .await?
                .take(0)?;
            if !existing.is_empty() {
                return Ok(());
            }
            db.query(
                "CREATE type::record('schedule', $rid) CONTENT {
                    namespace:        $ns,
                    name:             $sched_name,
                    workflow_type:    $workflow_type,
                    cron_expr:        $cron_expr,
                    timezone:         $timezone,
                    input:            $input,
                    task_queue:       $task_queue,
                    overlap_policy:   $overlap_policy,
                    paused:           $paused,
                    last_run_at:      $last_run_at,
                    next_run_at:      $next_run_at,
                    last_workflow_id: $last_workflow_id,
                    created_at:       $created_at
                }",
            )
            .bind(("rid", rid))
            .bind(("ns", s.namespace.clone()))
            .bind(("sched_name", s.name.clone()))
            .bind(("workflow_type", s.workflow_type.clone()))
            .bind(("cron_expr", s.cron_expr.clone()))
            .bind(("timezone", s.timezone.clone()))
            .bind(("input", s.input.clone()))
            .bind(("task_queue", s.task_queue.clone()))
            .bind(("overlap_policy", s.overlap_policy.clone()))
            .bind(("paused", s.paused))
            .bind(("last_run_at", s.last_run_at))
            .bind(("next_run_at", s.next_run_at))
            .bind(("last_workflow_id", s.last_workflow_id.clone()))
            .bind(("created_at", s.created_at))
            .await
            .map_err(|e| anyhow::anyhow!("create_schedule({}/{}): {e}", s.namespace, s.name))?;
            Ok(())
        }
    }

    pub(crate) fn get_schedule_impl(
        &self,
        namespace: &str,
        name: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowSchedule>>> + Send {
        let db = self.db.clone();
        let rid = schedule_rid(namespace, name);
        async move {
            let sql = format!(
                "SELECT {SELECT_FIELDS} FROM type::record('schedule', $rid)"
            );
            let rows: Vec<serde_json::Value> = db
                .query(&sql)
                .bind(("rid", rid))
                .await?
                .take(0)?;
            Ok(rows.into_iter().next().and_then(row_to_schedule))
        }
    }

    pub(crate) fn list_schedules_impl(
        &self,
        namespace: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowSchedule>>> + Send {
        let db = self.db.clone();
        let namespace = namespace.to_string();
        async move {
            let sql = format!(
                "SELECT {SELECT_FIELDS} FROM schedule WHERE namespace = $ns ORDER BY name ASC"
            );
            let rows: Vec<serde_json::Value> = db
                .query(&sql)
                .bind(("ns", namespace))
                .await?
                .take(0)?;
            Ok(rows.into_iter().filter_map(row_to_schedule).collect())
        }
    }

    pub(crate) fn update_schedule_last_run_impl(
        &self,
        namespace: &str,
        name: &str,
        last_run_at: f64,
        next_run_at: f64,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        let db = self.db.clone();
        let rid = schedule_rid(namespace, name);
        let workflow_id = workflow_id.to_string();
        async move {
            db.query(
                "UPDATE type::record('schedule', $rid) SET \
                 last_run_at = $last_run_at, \
                 next_run_at = $next_run_at, \
                 last_workflow_id = $last_wf_id",
            )
            .bind(("rid", rid))
            .bind(("last_run_at", last_run_at))
            .bind(("next_run_at", next_run_at))
            .bind(("last_wf_id", workflow_id))
            .await?;
            Ok(())
        }
    }

    pub(crate) fn delete_schedule_impl(
        &self,
        namespace: &str,
        name: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send {
        let db = self.db.clone();
        let rid = schedule_rid(namespace, name);
        async move {
            // Check existence first; DELETE on a missing record is silent in SurrealDB.
            let existing: Vec<serde_json::Value> = db
                .query("SELECT namespace FROM type::record('schedule', $rid) LIMIT 1")
                .bind(("rid", rid.clone()))
                .await?
                .take(0)?;
            if existing.is_empty() {
                return Ok(false);
            }
            db.query("DELETE type::record('schedule', $rid)")
                .bind(("rid", rid))
                .await?;
            Ok(true)
        }
    }

    pub(crate) fn update_schedule_impl(
        &self,
        namespace: &str,
        name: &str,
        patch: &SchedulePatch,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowSchedule>>> + Send {
        let db = self.db.clone();
        let rid = schedule_rid(namespace, name);
        let p = patch.clone();
        let namespace = namespace.to_string();
        let name = name.to_string();
        async move {
            // Build SET clause from whichever fields are Some.
            let mut sets: Vec<&str> = Vec::new();
            if p.cron_expr.is_some()     { sets.push("cron_expr = $cron_expr"); }
            if p.timezone.is_some()      { sets.push("timezone = $timezone"); }
            if p.input.is_some()         { sets.push("input = $input"); }
            if p.task_queue.is_some()    { sets.push("task_queue = $task_queue"); }
            if p.overlap_policy.is_some(){ sets.push("overlap_policy = $overlap_policy"); }

            if sets.is_empty() {
                // No-op — return current value.
                return Self::get_schedule_static(&db, &namespace, &name).await;
            }

            let sql = format!("UPDATE type::record('schedule', $rid) SET {}", sets.join(", "));
            let mut q = db.query(&sql).bind(("rid", rid));
            if let Some(ref v) = p.cron_expr      { q = q.bind(("cron_expr", v.clone())); }
            if let Some(ref v) = p.timezone        { q = q.bind(("timezone", v.clone())); }
            if let Some(ref v) = p.input           { q = q.bind(("input", v.to_string())); }
            if let Some(ref v) = p.task_queue      { q = q.bind(("task_queue", v.clone())); }
            if let Some(ref v) = p.overlap_policy  { q = q.bind(("overlap_policy", v.clone())); }

            let updated: Vec<serde_json::Value> = q.await?.take(0)?;
            if updated.is_empty() {
                return Ok(None);
            }

            Self::get_schedule_static(&db, &namespace, &name).await
        }
    }

    pub(crate) fn set_schedule_paused_impl(
        &self,
        namespace: &str,
        name: &str,
        paused: bool,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowSchedule>>> + Send {
        let db = self.db.clone();
        let rid = schedule_rid(namespace, name);
        let namespace = namespace.to_string();
        let name = name.to_string();
        async move {
            let updated: Vec<serde_json::Value> = db
                .query("UPDATE type::record('schedule', $rid) SET paused = $paused")
                .bind(("rid", rid))
                .bind(("paused", paused))
                .await?
                .take(0)?;
            if updated.is_empty() {
                return Ok(None);
            }
            Self::get_schedule_static(&db, &namespace, &name).await
        }
    }

    // ── Internal helper used by update_schedule_impl + set_schedule_paused_impl ─

    async fn get_schedule_static(
        db: &surrealdb::Surreal<surrealdb::engine::remote::ws::Client>,
        namespace: &str,
        name: &str,
    ) -> anyhow::Result<Option<WorkflowSchedule>> {
        let rid = schedule_rid(namespace, name);
        let sql = format!("SELECT {SELECT_FIELDS} FROM type::record('schedule', $rid)");
        let rows: Vec<serde_json::Value> = db
            .query(&sql)
            .bind(("rid", rid))
            .await?
            .take(0)?;
        Ok(rows.into_iter().next().and_then(row_to_schedule))
    }
}
