use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use utoipa::ToSchema;

// ── Workflow Status ─────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkflowStatus {
    Pending,
    Running,
    Waiting,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}

impl fmt::Display for WorkflowStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "PENDING"),
            Self::Running => write!(f, "RUNNING"),
            Self::Waiting => write!(f, "WAITING"),
            Self::Completed => write!(f, "COMPLETED"),
            Self::Failed => write!(f, "FAILED"),
            Self::Cancelled => write!(f, "CANCELLED"),
            Self::TimedOut => write!(f, "TIMED_OUT"),
        }
    }
}

impl FromStr for WorkflowStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PENDING" => Ok(Self::Pending),
            "RUNNING" => Ok(Self::Running),
            "WAITING" => Ok(Self::Waiting),
            "COMPLETED" => Ok(Self::Completed),
            "FAILED" => Ok(Self::Failed),
            "CANCELLED" => Ok(Self::Cancelled),
            "TIMED_OUT" => Ok(Self::TimedOut),
            _ => Err(format!("unknown workflow status: {s}")),
        }
    }
}

impl WorkflowStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }
}

// ── Activity Status ─────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActivityStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl fmt::Display for ActivityStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "PENDING"),
            Self::Running => write!(f, "RUNNING"),
            Self::Completed => write!(f, "COMPLETED"),
            Self::Failed => write!(f, "FAILED"),
            Self::Cancelled => write!(f, "CANCELLED"),
        }
    }
}

impl FromStr for ActivityStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PENDING" => Ok(Self::Pending),
            "RUNNING" => Ok(Self::Running),
            "COMPLETED" => Ok(Self::Completed),
            "FAILED" => Ok(Self::Failed),
            "CANCELLED" => Ok(Self::Cancelled),
            _ => Err(format!("unknown activity status: {s}")),
        }
    }
}

// ── Event Types ─────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    WorkflowStarted,
    ActivityScheduled,
    ActivityCompleted,
    ActivityFailed,
    TimerStarted,
    TimerFired,
    SignalReceived,
    WorkflowCompleted,
    WorkflowFailed,
    WorkflowCancelled,
    ChildWorkflowStarted,
    ChildWorkflowCompleted,
    SideEffectRecorded,
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| format!("{self:?}"));
        write!(f, "{s}")
    }
}

impl FromStr for EventType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(serde_json::Value::String(s.to_string()))
            .map_err(|_| format!("unknown event type: {s}"))
    }
}

// ── Overlap Policy ──────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapPolicy {
    Skip,
    Queue,
    CancelOld,
    AllowAll,
}

impl fmt::Display for OverlapPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Skip => write!(f, "skip"),
            Self::Queue => write!(f, "queue"),
            Self::CancelOld => write!(f, "cancel_old"),
            Self::AllowAll => write!(f, "allow_all"),
        }
    }
}

impl FromStr for OverlapPolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "skip" => Ok(Self::Skip),
            "queue" => Ok(Self::Queue),
            "cancel_old" => Ok(Self::CancelOld),
            "allow_all" => Ok(Self::AllowAll),
            _ => Err(format!("unknown overlap policy: {s}")),
        }
    }
}

// ── Records ─────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct WorkflowRecord {
    pub id: String,
    pub namespace: String,
    pub run_id: String,
    pub workflow_type: String,
    pub task_queue: String,
    pub status: String,
    pub input: Option<String>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub parent_id: Option<String>,
    pub claimed_by: Option<String>,
    pub created_at: f64,
    pub updated_at: f64,
    pub completed_at: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct WorkflowEvent {
    pub id: Option<i64>,
    pub workflow_id: String,
    pub seq: i32,
    pub event_type: String,
    pub payload: Option<String>,
    pub timestamp: f64,
}

/// Options for scheduling an activity. All fields default to sensible values
/// when not provided by the caller; this keeps the per-call API short while
/// still letting workflows tune retry/timeout policy when they need to.
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ScheduleActivityOpts {
    pub max_attempts: Option<i32>,
    pub initial_interval_secs: Option<f64>,
    pub backoff_coefficient: Option<f64>,
    pub start_to_close_secs: Option<f64>,
    pub heartbeat_timeout_secs: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct WorkflowActivity {
    pub id: Option<i64>,
    pub workflow_id: String,
    pub seq: i32,
    pub name: String,
    pub task_queue: String,
    pub input: Option<String>,
    pub status: String,
    pub result: Option<String>,
    pub error: Option<String>,
    pub attempt: i32,
    pub max_attempts: i32,
    pub initial_interval_secs: f64,
    pub backoff_coefficient: f64,
    pub start_to_close_secs: f64,
    pub heartbeat_timeout_secs: Option<f64>,
    pub claimed_by: Option<String>,
    pub scheduled_at: f64,
    pub started_at: Option<f64>,
    pub completed_at: Option<f64>,
    pub last_heartbeat: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct WorkflowTimer {
    pub id: Option<i64>,
    pub workflow_id: String,
    pub seq: i32,
    pub fire_at: f64,
    pub fired: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct WorkflowSignal {
    pub id: Option<i64>,
    pub workflow_id: String,
    pub name: String,
    pub payload: Option<String>,
    pub consumed: bool,
    pub received_at: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct WorkflowSchedule {
    pub name: String,
    pub namespace: String,
    pub workflow_type: String,
    pub cron_expr: String,
    /// IANA time-zone name used to interpret `cron_expr` (e.g. "Europe/Berlin",
    /// "America/New_York"). Defaults to "UTC" when a schedule is created
    /// without an explicit timezone, preserving v0.11.2 behaviour.
    pub timezone: String,
    pub input: Option<String>,
    pub task_queue: String,
    pub overlap_policy: String,
    pub paused: bool,
    pub last_run_at: Option<f64>,
    pub next_run_at: Option<f64>,
    pub last_workflow_id: Option<String>,
    pub created_at: f64,
}

/// Partial update to a `WorkflowSchedule`. Only fields set to `Some` are
/// applied; `None` leaves the existing value untouched. Used by
/// `PATCH /api/v1/schedules/{name}`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct SchedulePatch {
    pub cron_expr: Option<String>,
    pub timezone: Option<String>,
    pub input: Option<serde_json::Value>,
    pub task_queue: Option<String>,
    pub overlap_policy: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct WorkflowWorker {
    pub id: String,
    pub namespace: String,
    pub identity: String,
    pub task_queue: String,
    pub workflows: Option<String>,
    pub activities: Option<String>,
    pub max_concurrent_workflows: i32,
    pub max_concurrent_activities: i32,
    pub active_tasks: i32,
    pub last_heartbeat: f64,
    pub registered_at: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowSnapshot {
    pub workflow_id: String,
    pub event_seq: i32,
    pub state_json: String,
    pub created_at: f64,
}
