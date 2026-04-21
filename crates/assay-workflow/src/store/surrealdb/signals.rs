//! SurrealDB implementation of signal-related `WorkflowStore` methods.

use std::future::Future;

use assay_core::types::WorkflowSignal;

use super::SurrealDbStore;

// ── Helper ────────────────────────────────────────────────────────────────────

pub(super) fn row_to_signal(v: serde_json::Value) -> WorkflowSignal {
    WorkflowSignal {
        id: v.get("id_num").and_then(|x| x.as_i64()),
        workflow_id: v.get("workflow_id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        name: v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        // payload is stored as payload_str (string column) to mirror PG/SQLite
        payload: v.get("payload_str").and_then(|x| {
            if x.is_null() { None } else { x.as_str().map(|s| s.to_string()) }
        }),
        consumed: v.get("consumed").and_then(|x| x.as_bool()).unwrap_or(false),
        received_at: v.get("received_at").and_then(|x| x.as_f64()).unwrap_or(0.0),
    }
}

async fn next_signal_id(db: &surrealdb::Surreal<surrealdb::engine::remote::ws::Client>) -> anyhow::Result<i64> {
    let rows: Vec<serde_json::Value> = db
        .query("UPDATE _seq SET val = val + 1 WHERE name = $name RETURN val")
        .bind(("name", "signal".to_string()))
        .await?
        .take(0)?;
    rows.first()
        .and_then(|v| v.get("val"))
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("next_signal_id: counter row missing"))
}

// ── Signal method impls ───────────────────────────────────────────────────────

impl SurrealDbStore {
    pub(crate) fn send_signal_impl(
        &self,
        signal: &WorkflowSignal,
    ) -> impl Future<Output = anyhow::Result<i64>> + Send {
        let db = self.db.clone();
        let sig = signal.clone();
        async move {
            let id_num = next_signal_id(&db).await?;
            // Record key: use the id_num to guarantee uniqueness.
            let record_id = format!("sig_{id_num}");

            // `created_at` is required by the signal schema (TYPE float, no DEFAULT).
            // We populate it with the same value as `received_at`.
            db.query(
                "CREATE type::record('signal', $rid) CONTENT {
                    id_num:      $id_num,
                    workflow_id: $workflow_id,
                    name:        $sig_name,
                    payload_str: $payload,
                    consumed:    false,
                    received_at: $received_at,
                    created_at:  $received_at
                }",
            )
            .bind(("rid", record_id.clone()))
            .bind(("id_num", id_num))
            .bind(("workflow_id", sig.workflow_id.clone()))
            .bind(("sig_name", sig.name.clone()))
            .bind(("payload", sig.payload.clone()))
            .bind(("received_at", sig.received_at))
            .await
            .map_err(|e| anyhow::anyhow!("send_signal({}/{}): {e}", sig.workflow_id, sig.name))?
            .take::<Vec<serde_json::Value>>(0)
            .map_err(|e| anyhow::anyhow!("send_signal({}/{}): insert error: {e}", sig.workflow_id, sig.name))?;

            Ok(id_num)
        }
    }

    pub(crate) fn consume_signals_impl(
        &self,
        workflow_id: &str,
        name: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkflowSignal>>> + Send {
        let db = self.db.clone();
        let workflow_id = workflow_id.to_string();
        let name = name.to_string();
        async move {
            // Step 1: find all unconsumed signals matching this workflow+name.
            let pending: Vec<serde_json::Value> = db
                .query(
                    "SELECT id_num, workflow_id, name, payload_str, consumed, received_at
                     FROM signal
                     WHERE workflow_id = $wid AND name = $sig_name AND consumed = false",
                )
                .bind(("wid", workflow_id.clone()))
                .bind(("sig_name", name.clone()))
                .await?
                .take(0)?;

            if pending.is_empty() {
                return Ok(vec![]);
            }

            // Step 2: mark each as consumed by id_num.
            for row in &pending {
                if let Some(id_num) = row.get("id_num").and_then(|v| v.as_i64()) {
                    db.query("UPDATE signal SET consumed = true WHERE id_num = $id")
                        .bind(("id", id_num))
                        .await?;
                }
            }

            // Return the rows with consumed=true set.
            let result = pending
                .into_iter()
                .map(|row| {
                    let mut sig = row_to_signal(row);
                    sig.consumed = true;
                    sig
                })
                .collect();
            Ok(result)
        }
    }
}
