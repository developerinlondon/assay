//! Backend implementations of `WorkflowStore`.
//!
//! The trait itself lives in `assay-domain`; re-exported here so existing
//! `crate::store::WorkflowStore` and `crate::store::<DTO>` paths resolve
//! unchanged.

pub mod postgres;
pub mod sqlite;

pub use assay_domain::store::WorkflowStore;
pub use assay_domain::{NamespaceRecord, NamespaceStats, QueueStats};

use assay_domain::RetryFailedActivityResult;

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
