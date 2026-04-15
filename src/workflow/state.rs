use serde::{Deserialize, Serialize};

use crate::workflow::types::WorkflowStatus;

/// Commands yielded by a workflow execution turn.
/// The engine processes these to advance workflow state.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WorkflowCommand {
    ScheduleActivity {
        name: String,
        input: Option<String>,
        task_queue: Option<String>,
        #[serde(default = "default_max_attempts")]
        max_attempts: i32,
        #[serde(default = "default_initial_interval")]
        initial_interval_secs: f64,
        #[serde(default = "default_backoff")]
        backoff_coefficient: f64,
        #[serde(default = "default_start_to_close")]
        start_to_close_secs: f64,
        heartbeat_timeout_secs: Option<f64>,
    },
    StartTimer {
        duration_secs: f64,
    },
    CompleteWorkflow {
        result: Option<String>,
    },
    FailWorkflow {
        error: String,
    },
    StartChildWorkflow {
        workflow_type: String,
        workflow_id: String,
        input: Option<String>,
        task_queue: Option<String>,
    },
}

fn default_max_attempts() -> i32 {
    3
}
fn default_initial_interval() -> f64 {
    1.0
}
fn default_backoff() -> f64 {
    2.0
}
fn default_start_to_close() -> f64 {
    300.0
}

/// Validates whether a workflow status transition is legal.
pub fn is_valid_transition(from: WorkflowStatus, to: WorkflowStatus) -> bool {
    use WorkflowStatus::*;
    matches!(
        (from, to),
        // Normal forward transitions
        (Pending, Running)
            | (Running, Waiting)
            | (Running, Completed)
            | (Running, Failed)
            | (Waiting, Running)
            | (Waiting, Completed)
            | (Waiting, Failed)
            // Cancellation from any non-terminal state
            | (Pending, Cancelled)
            | (Running, Cancelled)
            | (Waiting, Cancelled)
            // Timeout from running/waiting
            | (Running, TimedOut)
            | (Waiting, TimedOut)
    )
}

/// Result of processing a single workflow execution turn.
#[derive(Debug)]
pub enum TurnResult {
    /// Workflow yielded commands and needs to continue
    Continue(Vec<WorkflowCommand>),
    /// Workflow completed with a result
    Completed(Option<String>),
    /// Workflow failed with an error
    Failed(String),
}
