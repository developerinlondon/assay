//! Backend implementations of `WorkflowStore`.
//!
//! The trait itself lives in `assay-domain`; re-exported here so existing
//! `crate::store::WorkflowStore` and `crate::store::<DTO>` paths resolve
//! unchanged.

pub mod postgres;
pub(crate) mod relocation;
pub mod sqlite;

pub use assay_domain::store::WorkflowStore;
pub use assay_domain::{NamespaceRecord, NamespaceStats, QueueStats};

use assay_domain::{RetryFailedActivityResult, SettleOutcome};

/// Marks a history event as not recording any activity's current
/// settlement: a payload the backfill could not read, or a settlement a
/// retry has superseded. Distinct from NULL, which the backfill revisits.
pub(crate) const NOT_A_SETTLEMENT: i64 = -1;

/// The `activity_id` a terminal activity event carries in its payload, or
/// `-1` when the payload predates the dedicated column or doesn't parse.
/// No activity has a negative id, so the sentinel is a durable "looked, and
/// there is nothing here" marker rather than a value the backfill revisits.
pub(crate) fn payload_activity_id(payload: Option<&str>) -> i64 {
    payload
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| value.get("activity_id").and_then(serde_json::Value::as_i64))
        .unwrap_or(NOT_A_SETTLEMENT)
}

pub(crate) fn settle_outcome(was_terminal: bool, event_present: bool) -> SettleOutcome {
    match (was_terminal, event_present) {
        (false, _) => SettleOutcome::Settled,
        (true, false) => SettleOutcome::Repaired,
        (true, true) => SettleOutcome::AlreadySettled,
    }
}

pub(crate) fn retry_denial(
    status: String,
    parent_id: Option<String>,
    archived_at: Option<f64>,
) -> Option<RetryFailedActivityResult> {
    if archived_at.is_some() {
        Some(RetryFailedActivityResult::Archived)
    } else if parent_id.is_some() {
        Some(RetryFailedActivityResult::ChildWorkflow)
    } else if status != "FAILED" {
        Some(RetryFailedActivityResult::NotFailed { status })
    } else {
        None
    }
}

pub(crate) struct RetryEvent<'a> {
    pub activity_id: i64,
    pub activity_seq: i32,
    pub activity_name: &'a str,
    pub failed_event_seq: i32,
    pub requested_by: &'a str,
    pub reason: &'a str,
    pub invalidated_activities: u64,
}

impl RetryEvent<'_> {
    pub fn payload(&self) -> serde_json::Value {
        serde_json::json!({
            "activity_id": self.activity_id,
            "activity_seq": self.activity_seq,
            "name": self.activity_name,
            "failed_event_seq": self.failed_event_seq,
            "requested_by": self.requested_by,
            "reason": self.reason,
            "invalidated_activities": self.invalidated_activities,
        })
    }
}
