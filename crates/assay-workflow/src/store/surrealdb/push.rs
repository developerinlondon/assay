//! SurrealDB push-stream implementations for `subscribe_runnable` and
//! `subscribe_tasks` via `LIVE SELECT`.
//!
//! Both streams use `async_stream::stream!` to drive the notification loop.
//! Transient errors are logged and skipped — the stream only terminates when
//! the underlying LIVE query channel closes (server restart / connection drop).
//!
//! Filter logic:
//!   - `subscribe_runnable`: yields workflow ids whose row was Created or
//!     Updated and that have `needs_dispatch = true` in PENDING or RUNNING status.
//!   - `subscribe_tasks`: yields activity `id_num` strings for rows Created
//!     with status = 'PENDING'.
//!
//! SQLite uses `stream::empty()` — no cross-process primitive available
//! (by design, plan 10 § "Dispatch wake-up — hybrid model").

use futures_util::StreamExt;
use surrealdb::types::Action;

use super::SurrealDbStore;

impl SurrealDbStore {
    /// Returns a stream that emits a workflow `id` string every time a
    /// workflow row that is dispatchable (needs_dispatch = true, status in
    /// PENDING/RUNNING) is inserted or updated.
    ///
    /// The id emitted is the bare workflow id (without the `workflow:` prefix).
    pub(crate) fn subscribe_runnable_impl(
        &self,
        namespace: &str,
    ) -> impl futures_core::Stream<Item = String> + Send + '_ {
        let db = self.db.clone();
        let ns = namespace.to_string();
        async_stream::stream! {
            // Issue the LIVE SELECT.
            // Note: LIVE SELECT in SurrealDB returns the full record on each
            // Create/Update/Delete notification. The WHERE clause filters which
            // records trigger notifications. We re-check `needs_dispatch` and
            // `status` in the Rust handler to guard against race conditions.
            let result = db
                .query(
                    "LIVE SELECT * FROM workflow \
                     WHERE namespace = $ns \
                       AND status IN ['PENDING', 'RUNNING'] \
                       AND needs_dispatch = true",
                )
                .bind(("ns", ns))
                .await;

            let mut response = match result {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(?e, "subscribe_runnable: LIVE query failed");
                    return;
                }
            };

            let mut live_stream = match response.stream::<surrealdb::types::Value>(0) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(?e, "subscribe_runnable: stream init failed");
                    return;
                }
            };

            while let Some(notif_result) = live_stream.next().await {
                match notif_result {
                    Ok(notif) => {
                        // Only fire on Create and Update; ignore Delete and Killed.
                        if !matches!(notif.action, Action::Create | Action::Update) {
                            continue;
                        }
                        // Extract the workflow id.
                        // The full record is in `notif.data` as Value::Object.
                        // The `id` field is a RecordId like `workflow:wf-xxx`.
                        // `into_json_value()` converts RecordId → "workflow:wf-xxx".
                        let id_str = extract_record_id_field(&notif.data, "id", "workflow");
                        match id_str {
                            Some(s) => yield s,
                            None => {
                                tracing::warn!(
                                    action = ?notif.action,
                                    data = ?notif.data,
                                    "subscribe_runnable: notification missing id field"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        // Transient error — log and continue; don't terminate the stream.
                        tracing::warn!(?e, "subscribe_runnable: notification error (skipped)");
                    }
                }
            }
        }
    }

    /// Returns a stream that emits an activity `id_num` string every time a
    /// PENDING activity is inserted on one of the watched task queues.
    pub(crate) fn subscribe_tasks_impl<'a>(
        &'a self,
        queue_names: &'a [&'a str],
    ) -> impl futures_core::Stream<Item = String> + Send + 'a {
        let db = self.db.clone();
        let queues: Vec<String> = queue_names.iter().map(|s| s.to_string()).collect();
        async_stream::stream! {
            if queues.is_empty() {
                return;
            }

            let result = db
                .query(
                    "LIVE SELECT * FROM activity \
                     WHERE task_queue IN $qs \
                       AND status = 'PENDING'",
                )
                .bind(("qs", queues))
                .await;

            let mut response = match result {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(?e, "subscribe_tasks: LIVE query failed");
                    return;
                }
            };

            let mut live_stream = match response.stream::<surrealdb::types::Value>(0) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(?e, "subscribe_tasks: stream init failed");
                    return;
                }
            };

            while let Some(notif_result) = live_stream.next().await {
                match notif_result {
                    Ok(notif) => {
                        // Only care about new inserts (Create); updates and
                        // deletes don't signal a new task being available.
                        if !matches!(notif.action, Action::Create) {
                            continue;
                        }
                        let id_str = extract_json_field_as_string(&notif.data, "id_num");
                        match id_str {
                            Some(s) => yield s,
                            None => {
                                tracing::warn!(
                                    action = ?notif.action,
                                    data = ?notif.data,
                                    "subscribe_tasks: notification missing id_num field"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(?e, "subscribe_tasks: notification error (skipped)");
                    }
                }
            }
        }
    }
}

// ── Field extraction helpers ──────────────────────────────────────────────────

/// Extract an `id` field that is a SurrealDB RecordId (`table:key`) and return
/// just the key part (stripping the `table_prefix:` prefix).
///
/// Falls back to returning the full string if it doesn't contain the prefix.
fn extract_record_id_field(
    data: &surrealdb::types::Value,
    field: &str,
    table_prefix: &str,
) -> Option<String> {
    let obj = match data {
        surrealdb::types::Value::Object(o) => o,
        _ => return None,
    };
    let val = obj.get(field)?.clone().into_json_value();
    let s = val.as_str()?;
    // SurrealDB RecordId serialises as "table:key" e.g. "workflow:wf-xxx".
    let prefix = format!("{table_prefix}:");
    if let Some(stripped) = s.strip_prefix(&prefix) {
        Some(stripped.to_string())
    } else {
        // Unexpected format — return the whole string so the caller gets something.
        Some(s.to_string())
    }
}

/// Extract any field from a Value::Object by converting it to JSON and
/// returning a string representation. Works for Number, String, etc.
fn extract_json_field_as_string(data: &surrealdb::types::Value, field: &str) -> Option<String> {
    let obj = match data {
        surrealdb::types::Value::Object(o) => o,
        _ => return None,
    };
    let val = obj.get(field)?.clone().into_json_value();
    if let Some(s) = val.as_str() {
        Some(s.to_string())
    } else if let Some(n) = val.as_i64() {
        Some(n.to_string())
    } else if let Some(n) = val.as_u64() {
        Some(n.to_string())
    } else if let Some(f) = val.as_f64() {
        // Truncate floats representing integers (id_num is stored as i64).
        Some((f as i64).to_string())
    } else {
        // Fallback: serialize the JSON value
        Some(val.to_string())
    }
}
