//! SurrealDB backend for `WorkflowStore`.
//!
//! `connect_full` connects to a remote SurrealDB instance over ws:// or wss://,
//! optionally signs in with Root credentials, selects the namespace + database,
//! and then runs the embedded SQL migrations (tracked via `_assay_migrations`).

mod migrations;
mod activities;
mod timers;
mod signals;
mod schedules;
mod snapshots;
mod workers;

use std::future::Future;

use assay_core::store::WorkflowStore;
use assay_core::types::*;
use surrealdb::engine::remote::ws::{Client, Ws, Wss};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;

pub struct SurrealDbStore {
    pub(crate) db: std::sync::Arc<Surreal<Client>>,
}

impl SurrealDbStore {
    /// Connect to a remote SurrealDB instance with full options.
    ///
    /// `url` must start with `ws://` or `wss://`.
    pub async fn connect_full(
        url: &str,
        namespace: &str,
        database: &str,
        username: Option<&str>,
        password: Option<&str>,
    ) -> anyhow::Result<Self> {
        let db: Surreal<Client> = if url.starts_with("wss://") {
            Surreal::new::<Wss>(url.trim_start_matches("wss://")).await?
        } else if url.starts_with("ws://") {
            Surreal::new::<Ws>(url.trim_start_matches("ws://")).await?
        } else {
            anyhow::bail!("SurrealDB DSN must start with ws:// or wss://")
        };

        if let (Some(u), Some(p)) = (username, password) {
            db.signin(Root {
                username: u.to_string(),
                password: p.to_string(),
            })
            .await?;
        }

        db.use_ns(namespace).use_db(database).await?;

        let this = Self {
            db: std::sync::Arc::new(db),
        };
        this.run_migrations().await?;
        Ok(this)
    }

    /// Convenience connect without auth, using default namespace/database.
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        Self::connect_full(url, "assay", "workflow", None, None).await
    }
}

// ── Helper utilities ─────────────────────────────────────────────────────────

fn timestamp_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// Convert a `serde_json::Value` (one row from a SurrealDB SELECT) to a
/// `WorkflowRecord`.  All fields are optional in the JSON — we use defaults
/// for missing ones so the conversion never fails.
fn row_to_workflow(v: serde_json::Value) -> WorkflowRecord {
    let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let namespace = v.get("namespace").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let run_id = v.get("run_id").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let workflow_type = v.get("workflow_type").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let task_queue = v.get("task_queue").and_then(|x| x.as_str()).unwrap_or("main").to_string();
    let status = v.get("status").and_then(|x| x.as_str()).unwrap_or("PENDING").to_string();
    let input = v.get("input").and_then(|x| if x.is_null() { None } else { x.as_str().map(|s| s.to_string()) });
    let result = v.get("result").and_then(|x| if x.is_null() { None } else { x.as_str().map(|s| s.to_string()) });
    let error = v.get("error").and_then(|x| if x.is_null() { None } else { x.as_str().map(|s| s.to_string()) });
    let parent_id = v.get("parent_id").and_then(|x| if x.is_null() { None } else { x.as_str().map(|s| s.to_string()) });
    let claimed_by = v.get("claimed_by").and_then(|x| if x.is_null() { None } else { x.as_str().map(|s| s.to_string()) });
    // search_attributes is a native object in SurrealDB; convert to JSON string.
    let search_attributes = v.get("search_attributes").and_then(|x| {
        if x.is_null() {
            None
        } else {
            Some(x.to_string())
        }
    });
    let archived_at = v.get("archived_at").and_then(|x| if x.is_null() { None } else { x.as_f64() });
    let archive_uri = v.get("archive_uri").and_then(|x| if x.is_null() { None } else { x.as_str().map(|s| s.to_string()) });
    let created_at = v.get("created_at").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let updated_at = v.get("updated_at").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let completed_at = v.get("completed_at").and_then(|x| if x.is_null() { None } else { x.as_f64() });

    WorkflowRecord {
        id,
        namespace,
        run_id,
        workflow_type,
        task_queue,
        status,
        input,
        result,
        error,
        parent_id,
        claimed_by,
        search_attributes,
        archived_at,
        archive_uri,
        created_at,
        updated_at,
        completed_at,
    }
}

/// Convert a serde_json::Value row to a WorkflowEvent.
fn row_to_event(v: serde_json::Value) -> WorkflowEvent {
    let workflow_id = v.get("workflow_id").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let seq = v.get("seq").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
    let event_type = v.get("event_type").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let payload = v.get("payload").and_then(|x| {
        if x.is_null() {
            None
        } else if let Some(s) = x.as_str() {
            Some(s.to_string())
        } else {
            // If stored as object, serialise back to string
            Some(x.to_string())
        }
    });
    let timestamp = v.get("created_at").and_then(|x| x.as_f64()).unwrap_or(0.0);
    WorkflowEvent {
        id: None,
        workflow_id,
        seq,
        event_type,
        payload,
        timestamp,
    }
}

// ── WorkflowStore impl ───────────────────────────────────────────────────────

impl WorkflowStore for SurrealDbStore {
    // ── Namespaces ─────────────────────────────────────────

    fn create_namespace(
        &self,
        name: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        let db = self.db.clone();
        let name = name.to_string();
        async move {
            let now = timestamp_now();
            db.query(
                "CREATE namespace CONTENT { name: $name, created_at: $created_at }",
            )
            .bind(("name", name))
            .bind(("created_at", now))
            .await?;
            Ok(())
        }
    }

    fn list_namespaces(
        &self,
    ) -> impl Future<Output = anyhow::Result<Vec<NamespaceRecord>>> + Send {
        let db = self.db.clone();
        async move {
            let rows: Vec<serde_json::Value> = db
                .query("SELECT name, created_at FROM namespace ORDER BY created_at ASC")
                .await?
                .take(0)?;
            let records = rows
                .into_iter()
                .filter_map(|v| {
                    let name = v.get("name")?.as_str()?.to_string();
                    let created_at = v.get("created_at")?.as_f64().unwrap_or(0.0);
                    Some(NamespaceRecord { name, created_at })
                })
                .collect();
            Ok(records)
        }
    }

    fn delete_namespace(
        &self,
        name: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send {
        let db = self.db.clone();
        let name = name.to_string();
        async move {
            // Protect the default "main" namespace — mirror PG behaviour.
            if name == "main" {
                return Ok(false);
            }
            let existing: Vec<serde_json::Value> = db
                .query("SELECT name FROM namespace WHERE name = $name LIMIT 1")
                .bind(("name", name.clone()))
                .await?
                .take(0)?;
            if existing.is_empty() {
                return Ok(false);
            }
            db.query("DELETE namespace WHERE name = $name")
                .bind(("name", name))
                .await?;
            Ok(true)
        }
    }

    fn get_namespace_stats(
        &self,
        namespace: &str,
    ) -> impl Future<Output = anyhow::Result<NamespaceStats>> + Send {
        let db = self.db.clone();
        let namespace = namespace.to_string();
        async move {
            let count_query = |status: Option<&str>| {
                if let Some(s) = status {
                    format!(
                        "SELECT count() AS c FROM workflow WHERE namespace = '{}' AND status = '{}' GROUP ALL",
                        namespace.replace('\'', "\\'"),
                        s
                    )
                } else {
                    format!(
                        "SELECT count() AS c FROM workflow WHERE namespace = '{}' GROUP ALL",
                        namespace.replace('\'', "\\'")
                    )
                }
            };

            let extract_count = |rows: Vec<serde_json::Value>| -> i64 {
                rows.first()
                    .and_then(|v| v.get("c"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
            };

            let total: i64 = extract_count(
                db.query(&count_query(None)).await?.take(0)?
            );
            let running: i64 = extract_count(
                db.query(&count_query(Some("RUNNING"))).await?.take(0)?
            );
            let pending: i64 = extract_count(
                db.query(&count_query(Some("PENDING"))).await?.take(0)?
            );
            let completed: i64 = extract_count(
                db.query(&count_query(Some("COMPLETED"))).await?.take(0)?
            );
            let failed: i64 = extract_count(
                db.query(&count_query(Some("FAILED"))).await?.take(0)?
            );

            let schedules: i64 = {
                let ns = namespace.replace('\'', "\\'");
                let rows: Vec<serde_json::Value> = db
                    .query(format!("SELECT count() AS c FROM schedule WHERE namespace = '{ns}' GROUP ALL"))
                    .await?.take(0)?;
                extract_count(rows)
            };

            let workers: i64 = {
                let ns = namespace.replace('\'', "\\'");
                let rows: Vec<serde_json::Value> = db
                    .query(format!("SELECT count() AS c FROM worker WHERE namespace = '{ns}' GROUP ALL"))
                    .await?.take(0)?;
                extract_count(rows)
            };

            Ok(NamespaceStats {
                namespace,
                total_workflows: total,
                running,
                pending,
                completed,
                failed,
                schedules,
                workers,
            })
        }
    }

    // ── Workflows ──────────────────────────────────────────

    fn create_workflow(
        &self,
        workflow: &WorkflowRecord,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        let db = self.db.clone();
        // Parse search_attributes from JSON string to Value so SurrealDB
        // stores it as a native object (matching the `option<object>` schema).
        let search_attributes: Option<serde_json::Value> = workflow
            .search_attributes
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok());
        let wf = workflow.clone();
        async move {
            db.query(
                "CREATE type::record('workflow', $id) CONTENT {
                    id:                      $id,
                    namespace:               $namespace,
                    run_id:                  $run_id,
                    workflow_type:           $workflow_type,
                    task_queue:              $task_queue,
                    status:                  $status,
                    input:                   $input,
                    result:                  $result,
                    error:                   $error,
                    parent_id:               $parent_id,
                    claimed_by:              $claimed_by,
                    search_attributes:       $search_attributes,
                    archived_at:             $archived_at,
                    archive_uri:             $archive_uri,
                    needs_dispatch:          false,
                    dispatch_claimed_by:     NONE,
                    dispatch_last_heartbeat: NONE,
                    created_at:              $created_at,
                    updated_at:              $updated_at,
                    completed_at:            $completed_at
                }",
            )
            .bind(("id", wf.id.clone()))
            .bind(("namespace", wf.namespace.clone()))
            .bind(("run_id", wf.run_id.clone()))
            .bind(("workflow_type", wf.workflow_type.clone()))
            .bind(("task_queue", wf.task_queue.clone()))
            .bind(("status", wf.status.clone()))
            .bind(("input", wf.input.clone()))
            .bind(("result", wf.result.clone()))
            .bind(("error", wf.error.clone()))
            .bind(("parent_id", wf.parent_id.clone()))
            .bind(("claimed_by", wf.claimed_by.clone()))
            .bind(("search_attributes", search_attributes))
            .bind(("archived_at", wf.archived_at))
            .bind(("archive_uri", wf.archive_uri.clone()))
            .bind(("created_at", wf.created_at))
            .bind(("updated_at", wf.updated_at))
            .bind(("completed_at", wf.completed_at))
            .await
            .map_err(|e| anyhow::anyhow!("create_workflow({}): {e}", wf.id))?;
            Ok(())
        }
    }

    fn get_workflow(
        &self,
        id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowRecord>>> + Send {
        let db = self.db.clone();
        let id = id.to_string();
        async move {
            let rows: Vec<serde_json::Value> = db
                .query("SELECT record::id(id) AS id, namespace, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, search_attributes, archived_at, archive_uri, needs_dispatch, dispatch_claimed_by, dispatch_last_heartbeat, created_at, updated_at, completed_at FROM type::record('workflow', $id)")
                .bind(("id", id))
                .await?
                .take(0)?;
            Ok(rows.into_iter().next().map(row_to_workflow))
        }
    }

    fn list_workflows(
        &self,
        namespace: &str,
        status: Option<WorkflowStatus>,
        workflow_type: Option<&str>,
        search_attrs_filter: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowRecord>>> + Send {
        let db = self.db.clone();
        let namespace = namespace.to_string();
        let status_str = status.map(|s| s.to_string());
        let workflow_type = workflow_type.map(|s| s.to_string());
        let filter_pairs: Vec<(String, serde_json::Value)> = search_attrs_filter
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v.as_object().cloned())
            .map(|m| m.into_iter().collect())
            .unwrap_or_default();
        async move {
            let mut conditions = vec!["namespace = $ns".to_string()];
            if status_str.is_some() {
                conditions.push("status = $status".to_string());
            }
            if workflow_type.is_some() {
                conditions.push("workflow_type = $wtype".to_string());
            }
            for (k, v) in &filter_pairs {
                let v_str = match v {
                    serde_json::Value::String(s) => format!("'{}'", s.replace('\'', "\\'")),
                    other => other.to_string(),
                };
                conditions.push(format!(
                    "search_attributes.`{}` = {}",
                    k.replace('`', "\\`"),
                    v_str
                ));
            }
            let where_clause = conditions.join(" AND ");
            let sql = format!(
                "SELECT record::id(id) AS id, namespace, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, search_attributes, archived_at, archive_uri, needs_dispatch, dispatch_claimed_by, dispatch_last_heartbeat, created_at, updated_at, completed_at FROM workflow WHERE {where_clause} ORDER BY created_at DESC LIMIT $limit START $offset"
            );
            let mut q = db
                .query(&sql)
                .bind(("ns", namespace))
                .bind(("limit", limit as u64))
                .bind(("offset", offset as u64));
            if let Some(s) = status_str {
                q = q.bind(("status", s));
            }
            if let Some(wt) = workflow_type {
                q = q.bind(("wtype", wt));
            }
            let rows: Vec<serde_json::Value> = q.await?.take(0)?;
            Ok(rows.into_iter().map(row_to_workflow).collect())
        }
    }

    fn list_archivable_workflows(
        &self,
        cutoff: f64,
        limit: i64,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowRecord>>> + Send {
        let db = self.db.clone();
        async move {
            let rows: Vec<serde_json::Value> = db
                .query(
                    "SELECT record::id(id) AS id, namespace, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, search_attributes, archived_at, archive_uri, needs_dispatch, dispatch_claimed_by, dispatch_last_heartbeat, created_at, updated_at, completed_at FROM workflow
                     WHERE status IN ['COMPLETED', 'FAILED', 'CANCELLED', 'TIMED_OUT']
                       AND completed_at != NONE
                       AND completed_at < $cutoff
                       AND archived_at = NONE
                     ORDER BY completed_at ASC
                     LIMIT $limit",
                )
                .bind(("cutoff", cutoff))
                .bind(("limit", limit as u64))
                .await?
                .take(0)?;
            Ok(rows.into_iter().map(row_to_workflow).collect())
        }
    }

    fn mark_archived_and_purge(
        &self,
        workflow_id: &str,
        archive_uri: &str,
        archived_at: f64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        let db = self.db.clone();
        let workflow_id = workflow_id.to_string();
        let archive_uri = archive_uri.to_string();
        async move {
            // SurrealDB doesn't have multi-statement transactions with ROLLBACK
            // over the WS protocol the same way PG does. We run each DELETE
            // individually. The workflow record update is last, so partial
            // cleanup is idempotent — a retry simply re-runs the DELETEs on
            // already-empty tables (no-ops) then updates the record.
            db.query("DELETE event WHERE workflow_id = $wid")
                .bind(("wid", workflow_id.clone()))
                .await?;
            db.query("DELETE activity WHERE workflow_id = $wid")
                .bind(("wid", workflow_id.clone()))
                .await?;
            db.query("DELETE timer WHERE workflow_id = $wid")
                .bind(("wid", workflow_id.clone()))
                .await?;
            db.query("DELETE signal WHERE workflow_id = $wid")
                .bind(("wid", workflow_id.clone()))
                .await?;
            db.query("DELETE snapshot WHERE workflow_id = $wid")
                .bind(("wid", workflow_id.clone()))
                .await?;
            db.query(
                "UPDATE type::record('workflow', $wid) SET archived_at = $archived_at, archive_uri = $archive_uri",
            )
            .bind(("wid", workflow_id))
            .bind(("archived_at", archived_at))
            .bind(("archive_uri", archive_uri))
            .await?;
            Ok(())
        }
    }

    fn upsert_search_attributes(
        &self,
        workflow_id: &str,
        patch_json: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        let db = self.db.clone();
        let workflow_id = workflow_id.to_string();
        let patch_json = patch_json.to_string();
        async move {
            // Read current search_attributes, merge the patch, write back.
            //
            // Concurrency note: this is a read-modify-write, not atomic. Two
            // concurrent upserts on the same workflow_id may produce a
            // lost-update. In practice search_attribute updates are called from
            // single-threaded workflow replays, so this is acceptable.
            let rows: Vec<serde_json::Value> = db
                .query("SELECT search_attributes FROM type::record('workflow', $wid)")
                .bind(("wid", workflow_id.clone()))
                .await?
                .take(0)?;

            let current_str: Option<String> = rows.first().and_then(|v| {
                let sa = v.get("search_attributes")?;
                if sa.is_null() {
                    None
                } else {
                    Some(sa.to_string())
                }
            });

            let merged_str = crate::store::sqlite::merge_search_attrs(
                current_str.as_deref(),
                &patch_json,
            )?;
            let merged_val: serde_json::Value = serde_json::from_str(&merged_str)
                .unwrap_or(serde_json::Value::Object(Default::default()));

            let _: Vec<serde_json::Value> = db
                .query(
                    "UPDATE type::record('workflow', $wid) SET search_attributes = $attrs",
                )
                .bind(("wid", workflow_id))
                .bind(("attrs", merged_val))
                .await?
                .take(0)?;
            Ok(())
        }
    }

    fn update_workflow_status(
        &self,
        id: &str,
        status: WorkflowStatus,
        result: Option<&str>,
        error: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        let db = self.db.clone();
        let id = id.to_string();
        let status_str = status.to_string();
        let result = result.map(|s| s.to_string());
        let error = error.map(|s| s.to_string());
        async move {
            let now = timestamp_now();
            let completed_at: Option<f64> = if status.is_terminal() { Some(now) } else { None };

            // Build a dynamic SET clause so we only overwrite non-None fields
            // (mirrors PG COALESCE behaviour).
            let mut sets = vec![
                "status = $status".to_string(),
                "updated_at = $updated_at".to_string(),
            ];
            if result.is_some() {
                sets.push("result = $result".to_string());
            }
            if error.is_some() {
                sets.push("error = $error".to_string());
            }
            if completed_at.is_some() {
                sets.push("completed_at = $completed_at".to_string());
            }
            let sql = format!(
                "UPDATE type::record('workflow', $id) SET {}",
                sets.join(", ")
            );
            let mut q = db
                .query(&sql)
                .bind(("id", id))
                .bind(("status", status_str))
                .bind(("updated_at", now));
            if let Some(r) = result {
                q = q.bind(("result", r));
            }
            if let Some(e) = error {
                q = q.bind(("error", e));
            }
            if let Some(ca) = completed_at {
                q = q.bind(("completed_at", ca));
            }
            q.await?;
            Ok(())
        }
    }

    fn claim_workflow(
        &self,
        id: &str,
        worker_id: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send {
        let db = self.db.clone();
        let id = id.to_string();
        let worker_id = worker_id.to_string();
        async move {
            // Optimistic claim: only update if claimed_by is currently NONE.
            // We use RETURN BEFORE to detect whether the WHERE matched.
            //
            // SurrealDB lacks FOR UPDATE SKIP LOCKED; under a race, the last
            // writer wins the field value, but we detect the claim failure via
            // the pre-update state. See plan 10 § "Transactions and concurrency"
            // for a future transaction-based improvement.
            let now = timestamp_now();
            let rows: Vec<serde_json::Value> = db
                .query(
                    "UPDATE type::record('workflow', $id)
                     SET claimed_by = $worker, status = 'RUNNING', updated_at = $now
                     WHERE claimed_by = NONE
                     RETURN BEFORE",
                )
                .bind(("id", id))
                .bind(("worker", worker_id))
                .bind(("now", now))
                .await?
                .take(0)?;
            Ok(!rows.is_empty())
        }
    }

    fn mark_workflow_dispatchable(
        &self,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        let db = self.db.clone();
        let workflow_id = workflow_id.to_string();
        async move {
            db.query(
                "UPDATE type::record('workflow', $id) SET needs_dispatch = true",
            )
            .bind(("id", workflow_id))
            .await?;
            Ok(())
        }
    }

    fn claim_workflow_task(
        &self,
        task_queue: &str,
        worker_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowRecord>>> + Send {
        let db = self.db.clone();
        let task_queue = task_queue.to_string();
        let worker_id = worker_id.to_string();
        async move {
            // Step 1: find the oldest dispatchable workflow on this queue.
            let candidates: Vec<serde_json::Value> = db
                .query(
                    "SELECT record::id(id) AS id, namespace, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, search_attributes, archived_at, archive_uri, needs_dispatch, dispatch_claimed_by, dispatch_last_heartbeat, created_at, updated_at, completed_at FROM workflow
                     WHERE task_queue = $tq
                       AND needs_dispatch = true
                       AND dispatch_claimed_by = NONE
                       AND status NOT IN ['COMPLETED', 'FAILED', 'CANCELLED', 'TIMED_OUT']
                     ORDER BY updated_at ASC
                     LIMIT 1",
                )
                .bind(("tq", task_queue))
                .await?
                .take(0)?;

            let candidate = match candidates.into_iter().next() {
                Some(c) => c,
                None => return Ok(None),
            };

            let wf_id = candidate
                .get("id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if wf_id.is_empty() {
                return Ok(None);
            }

            let now = timestamp_now();

            // Step 2: atomically claim using a conditional UPDATE WHERE.
            // RETURN AFTER gives us the updated row only when the WHERE matched.
            let updated: Vec<serde_json::Value> = db
                .query(
                    "UPDATE type::record('workflow', $id)
                     SET dispatch_claimed_by = $worker,
                         dispatch_last_heartbeat = $now,
                         needs_dispatch = false
                     WHERE dispatch_claimed_by = NONE
                     RETURN record::id(id) AS id, namespace, run_id, workflow_type, task_queue, status, input, result, error, parent_id, claimed_by, search_attributes, archived_at, archive_uri, needs_dispatch, dispatch_claimed_by, dispatch_last_heartbeat, created_at, updated_at, completed_at",
                )
                .bind(("id", wf_id))
                .bind(("worker", worker_id))
                .bind(("now", now))
                .await?
                .take(0)?;

            Ok(updated.into_iter().next().map(row_to_workflow))
        }
    }

    fn release_workflow_task(
        &self,
        workflow_id: &str,
        worker_id: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        let db = self.db.clone();
        let workflow_id = workflow_id.to_string();
        let worker_id = worker_id.to_string();
        async move {
            db.query(
                "UPDATE type::record('workflow', $id)
                 SET dispatch_claimed_by = NONE, dispatch_last_heartbeat = NONE
                 WHERE dispatch_claimed_by = $worker",
            )
            .bind(("id", workflow_id))
            .bind(("worker", worker_id))
            .await?;
            Ok(())
        }
    }

    fn release_stale_dispatch_leases(
        &self,
        now: f64,
        timeout_secs: f64,
    ) -> impl Future<Output = anyhow::Result<u64>> + Send {
        let db = self.db.clone();
        async move {
            // Count first, then update.
            let stale: Vec<serde_json::Value> = db
                .query(
                    "SELECT id FROM workflow
                     WHERE dispatch_claimed_by != NONE
                       AND ($now - dispatch_last_heartbeat) > $timeout
                       AND status NOT IN ['COMPLETED', 'FAILED', 'CANCELLED', 'TIMED_OUT']",
                )
                .bind(("now", now))
                .bind(("timeout", timeout_secs))
                .await?
                .take(0)?;
            let count = stale.len() as u64;
            if count > 0 {
                db.query(
                    "UPDATE workflow
                     SET dispatch_claimed_by = NONE,
                         dispatch_last_heartbeat = NONE,
                         needs_dispatch = true
                     WHERE dispatch_claimed_by != NONE
                       AND ($now - dispatch_last_heartbeat) > $timeout
                       AND status NOT IN ['COMPLETED', 'FAILED', 'CANCELLED', 'TIMED_OUT']",
                )
                .bind(("now", now))
                .bind(("timeout", timeout_secs))
                .await?;
            }
            Ok(count)
        }
    }

    // ── Events ─────────────────────────────────────────────

    fn append_event(
        &self,
        event: &WorkflowEvent,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send {
        let db = self.db.clone();
        let ev = event.clone();
        async move {
            // Use "{workflow_id}_{seq}" as the record ID — seq is already a
            // monotonic unique counter per workflow.
            let record_id = format!("{}_{}", ev.workflow_id, ev.seq);
            db.query(
                "CREATE type::record('event', $eid) CONTENT {
                    workflow_id: $workflow_id,
                    seq:         $seq,
                    event_type:  $event_type,
                    payload:     $payload,
                    created_at:  $created_at
                }",
            )
            .bind(("eid", record_id))
            .bind(("workflow_id", ev.workflow_id.clone()))
            .bind(("seq", ev.seq))
            .bind(("event_type", ev.event_type.clone()))
            .bind(("payload", ev.payload.clone()))
            .bind(("created_at", ev.timestamp))
            .await
            .map_err(|e| anyhow::anyhow!("append_event({}:{}): {e}", ev.workflow_id, ev.seq))?;
            // Return seq cast to i64 as a synthetic primary-key equivalent.
            Ok(ev.seq as i64)
        }
    }

    fn list_events(
        &self,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowEvent>>> + Send {
        let db = self.db.clone();
        let workflow_id = workflow_id.to_string();
        async move {
            let rows: Vec<serde_json::Value> = db
                .query(
                    "SELECT workflow_id, seq, event_type, payload, created_at
                     FROM event WHERE workflow_id = $wid ORDER BY seq ASC",
                )
                .bind(("wid", workflow_id))
                .await?
                .take(0)?;
            Ok(rows.into_iter().map(row_to_event).collect())
        }
    }

    fn get_event_count(
        &self,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send {
        let db = self.db.clone();
        let workflow_id = workflow_id.to_string();
        async move {
            let rows: Vec<serde_json::Value> = db
                .query(
                    "SELECT count() AS c FROM event WHERE workflow_id = $wid GROUP ALL",
                )
                .bind(("wid", workflow_id))
                .await?
                .take(0)?;
            Ok(rows
                .first()
                .and_then(|v| v.get("c"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0))
        }
    }

    // ── Activities (Task 3.6) ─────────────────────────────────

    fn create_activity(
        &self,
        activity: &WorkflowActivity,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send {
        self.create_activity_impl(activity)
    }

    fn get_activity(
        &self,
        id: i64,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowActivity>>> + Send {
        self.get_activity_impl(id)
    }

    fn get_activity_by_workflow_seq(
        &self,
        workflow_id: &str,
        seq: i32,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowActivity>>> + Send {
        self.get_activity_by_workflow_seq_impl(workflow_id, seq)
    }

    fn claim_activity(
        &self,
        task_queue: &str,
        worker_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowActivity>>> + Send {
        self.claim_activity_impl(task_queue, worker_id)
    }

    fn requeue_activity_for_retry(
        &self,
        id: i64,
        next_attempt: i32,
        next_scheduled_at: f64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        self.requeue_activity_for_retry_impl(id, next_attempt, next_scheduled_at)
    }

    fn complete_activity(
        &self,
        id: i64,
        result: Option<&str>,
        error: Option<&str>,
        failed: bool,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        self.complete_activity_impl(id, result, error, failed)
    }

    fn heartbeat_activity(
        &self,
        id: i64,
        details: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        self.heartbeat_activity_impl(id, details)
    }

    fn get_timed_out_activities(
        &self,
        now: f64,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowActivity>>> + Send {
        self.get_timed_out_activities_impl(now)
    }

    fn cancel_pending_activities(
        &self,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<u64>> + Send {
        self.cancel_pending_activities_impl(workflow_id)
    }

    // ── Timers (Task 3.7) ─────────────────────────────────────

    fn cancel_pending_timers(
        &self,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<u64>> + Send {
        self.cancel_pending_timers_impl(workflow_id)
    }

    fn create_timer(
        &self,
        timer: &WorkflowTimer,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send {
        self.create_timer_impl(timer)
    }

    fn get_timer_by_workflow_seq(
        &self,
        workflow_id: &str,
        seq: i32,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowTimer>>> + Send {
        self.get_timer_by_workflow_seq_impl(workflow_id, seq)
    }

    fn fire_due_timers(
        &self,
        now: f64,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowTimer>>> + Send {
        self.fire_due_timers_impl(now)
    }

    // ── Signals (Task 3.8) ────────────────────────────────────

    fn send_signal(
        &self,
        signal: &WorkflowSignal,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send {
        self.send_signal_impl(signal)
    }

    fn consume_signals(
        &self,
        workflow_id: &str,
        name: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowSignal>>> + Send {
        self.consume_signals_impl(workflow_id, name)
    }

    fn create_schedule(
        &self,
        schedule: &WorkflowSchedule,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        self.create_schedule_impl(schedule)
    }

    fn get_schedule(
        &self,
        namespace: &str,
        name: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowSchedule>>> + Send {
        self.get_schedule_impl(namespace, name)
    }

    fn list_schedules(
        &self,
        namespace: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowSchedule>>> + Send {
        self.list_schedules_impl(namespace)
    }

    fn update_schedule_last_run(
        &self,
        namespace: &str,
        name: &str,
        last_run_at: f64,
        next_run_at: f64,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        self.update_schedule_last_run_impl(namespace, name, last_run_at, next_run_at, workflow_id)
    }

    fn delete_schedule(
        &self,
        namespace: &str,
        name: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send {
        self.delete_schedule_impl(namespace, name)
    }

    fn update_schedule(
        &self,
        namespace: &str,
        name: &str,
        patch: &SchedulePatch,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowSchedule>>> + Send {
        self.update_schedule_impl(namespace, name, patch)
    }

    fn set_schedule_paused(
        &self,
        namespace: &str,
        name: &str,
        paused: bool,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowSchedule>>> + Send {
        self.set_schedule_paused_impl(namespace, name, paused)
    }

    fn register_worker(
        &self,
        worker: &WorkflowWorker,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        self.register_worker_impl(worker)
    }

    fn heartbeat_worker(
        &self,
        id: &str,
        now: f64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        self.heartbeat_worker_impl(id, now)
    }

    fn list_workers(
        &self,
        namespace: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowWorker>>> + Send {
        self.list_workers_impl(namespace)
    }

    fn remove_dead_workers(
        &self,
        cutoff: f64,
    ) -> impl Future<Output = anyhow::Result<Vec<String>>> + Send {
        self.remove_dead_workers_impl(cutoff)
    }

    fn create_api_key(
        &self,
        _key_hash: &str,
        _prefix: &str,
        _label: Option<&str>,
        _created_at: f64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { todo!("Task 3.13") }
    }

    fn validate_api_key(
        &self,
        _key_hash: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send {
        async { todo!("Task 3.13") }
    }

    fn list_api_keys(
        &self,
    ) -> impl Future<Output = anyhow::Result<Vec<ApiKeyRecord>>> + Send {
        async { todo!("Task 3.13") }
    }

    fn revoke_api_key(
        &self,
        _prefix: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send {
        async { todo!("Task 3.13") }
    }

    fn api_keys_empty(&self) -> impl Future<Output = anyhow::Result<bool>> + Send {
        async { todo!("Task 3.13") }
    }

    fn get_api_key_by_label(
        &self,
        _label: &str,
    ) -> impl Future<Output = anyhow::Result<Option<ApiKeyRecord>>> + Send {
        async { todo!("Task 3.13") }
    }

    fn list_child_workflows(
        &self,
        _parent_id: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowRecord>>> + Send {
        async { todo!("Task 3.14") }
    }

    fn create_snapshot(
        &self,
        workflow_id: &str,
        event_seq: i32,
        state_json: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        self.create_snapshot_impl(workflow_id, event_seq, state_json)
    }

    fn get_latest_snapshot(
        &self,
        workflow_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WorkflowSnapshot>>> + Send {
        self.get_latest_snapshot_impl(workflow_id)
    }

    fn get_queue_stats(
        &self,
        _namespace: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<QueueStats>>> + Send {
        async { todo!("Task 3.14") }
    }

    fn try_acquire_scheduler_lock(
        &self,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send {
        async { todo!("Task 3.15") }
    }

    fn subscribe_runnable(
        &self,
        _namespace: &str,
    ) -> impl futures_core::Stream<Item = String> + Send + '_ {
        futures_util::stream::empty()
    }

    fn subscribe_tasks<'a>(
        &'a self,
        _queue_names: &'a [&'a str],
    ) -> impl futures_core::Stream<Item = String> + Send + 'a {
        futures_util::stream::empty()
    }
}
