//! SurrealDB leader-election implementation (Task 3.15).
//!
//! ## Semantics
//!
//! Unlike Postgres `pg_try_advisory_lock(42)` which is session-scoped and
//! automatically released when the connection drops, SurrealDB uses a
//! `scheduler_lock` record with an `expires_at` timestamp.
//!
//! ### Lock acquisition logic
//!
//! 1. SELECT the single `scheduler_lock:main` record (if it exists).
//! 2. If no record exists → CREATE it with `holder = $holder` and
//!    `expires_at = now + 60s` → return **true**.
//! 3. If a record exists and `expires_at > now` (lock is live):
//!    - If `holder == $holder` → refresh `expires_at` → return **true**
//!      (same instance re-acquires / heartbeats its own lock).
//!    - If `holder != $holder` → return **false** (another instance holds it).
//! 4. If a record exists and `expires_at <= now` (lock has expired) →
//!    UPDATE with new holder + expires_at → return **true**.
//!
//! ### Same-instance semantics
//!
//! Calling `try_acquire_scheduler_lock` twice from the same `SurrealDbStore`
//! instance (same `holder` string derived from process identity) returns
//! **true** both times and refreshes the TTL. This matches the PG advisory
//! lock behaviour where the same connection re-acquires without blocking.
//!
//! In the smoke test we use two calls from the same store (single process),
//! so both return `true`. The test documents this in the assertion comment.
//!
//! ### Lock TTL
//!
//! 60 seconds — matching the conceptual "scheduler heartbeat" interval.
//! The holder should call `try_acquire_scheduler_lock` at least once per
//! 60 s to prevent expiry and loss of leadership.

use std::future::Future;

use super::{timestamp_now, SurrealDbStore};

/// Fixed record ID for the global scheduler lock.
const LOCK_ID: &str = "main";
/// Holder name — unique per process.  Uses the process ID so multiple
/// replicas on the same host get different identities.
fn holder_id() -> String {
    format!("assay-scheduler-{}", std::process::id())
}
/// Lock TTL in seconds.
const LOCK_TTL_SECS: f64 = 60.0;

impl SurrealDbStore {
    pub(crate) fn try_acquire_scheduler_lock_impl(
        &self,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send {
        let db = self.db.clone();
        async move {
            let now = timestamp_now();
            let expires_at = now + LOCK_TTL_SECS;
            let holder = holder_id();

            // Read current lock record.
            let rows: Vec<serde_json::Value> = db
                .query(
                    "SELECT holder, expires_at FROM type::record('scheduler_lock', $lid)",
                )
                .bind(("lid", LOCK_ID))
                .await?
                .take(0)?;

            if rows.is_empty() {
                // No lock record — create it.
                db.query(
                    "CREATE type::record('scheduler_lock', $lid) CONTENT {
                        holder:     $holder,
                        expires_at: $expires_at
                    }",
                )
                .bind(("lid", LOCK_ID))
                .bind(("holder", holder))
                .bind(("expires_at", expires_at))
                .await
                .map_err(|e| anyhow::anyhow!("try_acquire_scheduler_lock CREATE: {e}"))?;
                return Ok(true);
            }

            let row = &rows[0];
            let current_holder = row
                .get("holder")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let current_expires = row
                .get("expires_at")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);

            if current_expires > now {
                // Lock is live.
                if current_holder == holder {
                    // Same instance — refresh TTL.
                    db.query(
                        "UPDATE type::record('scheduler_lock', $lid) SET expires_at = $expires_at",
                    )
                    .bind(("lid", LOCK_ID))
                    .bind(("expires_at", expires_at))
                    .await?;
                    return Ok(true);
                }
                // Another instance holds a live lock.
                return Ok(false);
            }

            // Lock has expired — take it over.
            db.query(
                "UPDATE type::record('scheduler_lock', $lid) SET holder = $holder, expires_at = $expires_at",
            )
            .bind(("lid", LOCK_ID))
            .bind(("holder", holder))
            .bind(("expires_at", expires_at))
            .await?;
            Ok(true)
        }
    }
}
