//! Typed workflow event layer on top of `assay_domain::events`.
//!
//! Every state-mutating method in `assay-workflow` that previously
//! called `ctx.broadcast(...)` or relied on a PG trigger for
//! `pg_notify` now emits a `WorkflowBusEvent` via `WorkflowEventBus`.

use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use assay_domain::events::{
    CursorGoneError, EngineEventBus, Event, EventFilter, NewEvent, Subsystem,
};

/// Every event kind the workflow subsystem emits. Each variant
/// serialises to a `workflow_*` / `activity_*` kind string + a JSON
/// payload. Fields are chosen to be small and self-contained; we never
/// dump whole rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowBusEvent {
    WorkflowCreated {
        workflow_id: String,
        workflow_type: String,
        task_queue: String,
        status: String,
    },
    WorkflowStatusChanged {
        workflow_id: String,
        old_status: String,
        new_status: String,
        task_queue: String,
    },
    WorkflowNeedsDispatch {
        workflow_id: String,
        task_queue: String,
    },
    WorkflowStarted {
        workflow_id: String,
    },
    WorkflowRunning {
        workflow_id: String,
    },
    WorkflowCompleted {
        workflow_id: String,
    },
    WorkflowFailed {
        workflow_id: String,
    },
    WorkflowCancelled {
        workflow_id: String,
    },
    WorkflowTerminated {
        workflow_id: String,
    },
    ActivityInserted {
        activity_id: i64,
        workflow_id: String,
        task_queue: String,
        name: String,
    },
    ActivityStatusChanged {
        activity_id: i64,
        workflow_id: String,
        old_status: String,
        new_status: String,
    },
    SignalReceived {
        workflow_id: String,
        signal_name: String,
    },
    TimerFired {
        workflow_id: String,
        seq: i32,
    },
}

impl WorkflowBusEvent {
    /// The kind string written to `engine_events.kind`. Matches the
    /// serde `rename_all = "snake_case"` tag for each variant.
    pub fn kind(&self) -> &'static str {
        match self {
            WorkflowBusEvent::WorkflowCreated { .. } => "workflow_created",
            WorkflowBusEvent::WorkflowStatusChanged { .. } => "workflow_status_changed",
            WorkflowBusEvent::WorkflowNeedsDispatch { .. } => "workflow_needs_dispatch",
            WorkflowBusEvent::WorkflowStarted { .. } => "workflow_started",
            WorkflowBusEvent::WorkflowRunning { .. } => "workflow_running",
            WorkflowBusEvent::WorkflowCompleted { .. } => "workflow_completed",
            WorkflowBusEvent::WorkflowFailed { .. } => "workflow_failed",
            WorkflowBusEvent::WorkflowCancelled { .. } => "workflow_cancelled",
            WorkflowBusEvent::WorkflowTerminated { .. } => "workflow_terminated",
            WorkflowBusEvent::ActivityInserted { .. } => "activity_inserted",
            WorkflowBusEvent::ActivityStatusChanged { .. } => "activity_status_changed",
            WorkflowBusEvent::SignalReceived { .. } => "signal_received",
            WorkflowBusEvent::TimerFired { .. } => "timer_fired",
        }
    }

    /// Serialise the variant's fields to JSON for `engine_events.payload`.
    /// Strips the `kind` serde tag from the payload since the kind
    /// lives in its own column.
    pub fn payload(&self) -> serde_json::Value {
        let v = serde_json::to_value(self).expect("WorkflowBusEvent serialisable");
        match v {
            serde_json::Value::Object(mut m) => {
                m.remove("kind");
                serde_json::Value::Object(m)
            }
            other => other,
        }
    }
}

/// Typed wrapper around an `EngineEventBus`. Per-subsystem wrappers
/// (workflow, auth, secrets) share the underlying bus instance at the
/// engine level but produce/consume their own typed events.
#[derive(Clone)]
pub struct WorkflowEventBus {
    inner: Arc<dyn EngineEventBus>,
}

impl WorkflowEventBus {
    pub fn new(inner: Arc<dyn EngineEventBus>) -> Self {
        Self { inner }
    }

    /// Publish a typed workflow event. `namespace` is the owning
    /// workflow's namespace.
    pub async fn publish(&self, namespace: &str, ev: WorkflowBusEvent) -> Result<i64> {
        let kind = ev.kind();
        let payload = ev.payload();
        self.inner
            .publish_committed(NewEvent {
                namespace,
                subsystem: Subsystem::Workflow,
                kind,
                payload,
            })
            .await
    }

    /// Read a cursor's worth of events for this namespace (any
    /// subsystem — SSE uses this for the replay phase).
    pub async fn read_since(
        &self,
        namespace: &str,
        after: Option<i64>,
        filter: &EventFilter,
        limit: u32,
    ) -> std::result::Result<Vec<Event>, CursorGoneError> {
        self.inner.read_since(namespace, after, filter, limit).await
    }

    /// Expose the underlying bus so the scheduler + SSE can subscribe
    /// at the generic level and filter for any subsystem.
    pub fn inner(&self) -> Arc<dyn EngineEventBus> {
        Arc::clone(&self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_tag_stripped_from_payload() {
        let e = WorkflowBusEvent::WorkflowCreated {
            workflow_id: "wf-1".into(),
            workflow_type: "greet".into(),
            task_queue: "main".into(),
            status: "PENDING".into(),
        };
        assert_eq!(e.kind(), "workflow_created");
        let payload = e.payload();
        assert_eq!(payload["workflow_id"], "wf-1");
        assert_eq!(payload["workflow_type"], "greet");
        assert!(payload.get("kind").is_none(), "kind tag must be stripped");
    }

    #[test]
    fn activity_inserted_payload() {
        let e = WorkflowBusEvent::ActivityInserted {
            activity_id: 42,
            workflow_id: "wf-1".into(),
            task_queue: "main".into(),
            name: "send_email".into(),
        };
        let p = e.payload();
        assert_eq!(p["activity_id"], 42);
        assert_eq!(p["workflow_id"], "wf-1");
    }
}
