use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Subsystem that produced an event. Stored as a short string in
/// `engine_events.subsystem` so we can filter server-side without
/// touching JSON payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Subsystem {
    Workflow,
    Auth,
    Secrets,
    System,
}

impl Subsystem {
    pub fn as_str(&self) -> &'static str {
        match self {
            Subsystem::Workflow => "workflow",
            Subsystem::Auth => "auth",
            Subsystem::Secrets => "secrets",
            Subsystem::System => "system",
        }
    }

    pub fn from_str(s: &str) -> Subsystem {
        match s {
            "workflow" => Subsystem::Workflow,
            "auth" => Subsystem::Auth,
            "secrets" => Subsystem::Secrets,
            _ => Subsystem::System,
        }
    }
}

/// One row from `engine_events`. The `payload` is subsystem-specific
/// JSON (deserialised by the subsystem wrapper into e.g. `WorkflowEvent`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: i64,
    pub ts: f64,
    pub namespace: String,
    pub subsystem: Subsystem,
    pub kind: String,
    pub payload: serde_json::Value,
}

/// A new event being written. Caller supplies everything except `id` /
/// `ts` — the impl stamps those.
#[derive(Debug, Clone)]
pub struct NewEvent<'a> {
    pub namespace: &'a str,
    pub subsystem: Subsystem,
    pub kind: &'a str,
    pub payload: serde_json::Value,
}

/// Filter applied server-side before an event is sent to a subscriber.
/// Empty vecs / `None`s mean "no filter on this dimension".
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub subsystems: Vec<Subsystem>,
    pub kinds: Vec<String>,
    pub workflow_id: Option<String>,
}

impl EventFilter {
    pub fn matches(&self, e: &Event) -> bool {
        if !self.subsystems.is_empty() && !self.subsystems.contains(&e.subsystem) {
            return false;
        }
        if !self.kinds.is_empty() && !self.kinds.iter().any(|k| k == &e.kind) {
            return false;
        }
        if let Some(ref wf_id) = self.workflow_id {
            if e.payload.get("workflow_id").and_then(|v| v.as_str()) != Some(wf_id) {
                return false;
            }
        }
        true
    }
}

/// Error returned when a subscriber's cursor is older than the retention
/// window — callers must resync via a point query + resubscribe.
#[derive(Debug, thiserror::Error)]
#[error("cursor {after} is older than retention window (oldest id: {oldest})")]
pub struct CursorGoneError {
    pub after: i64,
    pub oldest: i64,
}

/// The engine-wide event bus. Implementations exist per backend
/// (`PgEngineEventBus`, `SqliteEngineEventBus`) and are constructed at
/// engine startup alongside the `WorkflowStore`.
#[async_trait::async_trait]
pub trait EngineEventBus: Send + Sync + 'static {
    /// Append an event to the outbox and publish it. For PG this is a
    /// single transaction containing `INSERT engine_events ... RETURNING id`
    /// + `pg_notify(channel, id)` so the commit atomically publishes
    /// the event. For SQLite this is a bare INSERT + local broadcast
    /// send.
    ///
    /// Returns the assigned `id`.
    async fn publish_committed(&self, ev: NewEvent<'_>) -> Result<i64>;

    /// Read events strictly greater than `after` in the given namespace.
    /// Applies `filter` server-side. Returns up to `limit` events
    /// ordered by `id ASC`. Caller uses `.last().id` as the next cursor.
    ///
    /// If `after` is older than retention, returns `Err(CursorGoneError)`
    /// so the SSE layer can translate to HTTP 410.
    async fn read_since(
        &self,
        namespace: &str,
        after: Option<i64>,
        filter: &EventFilter,
        limit: u32,
    ) -> std::result::Result<Vec<Event>, CursorGoneError>;

    /// Subscribe to newly-published events on this node. The returned
    /// receiver yields events as they're published by same-node emits
    /// or (on PG) by the LISTEN bridge. `tokio::broadcast::Lagged`
    /// errors reach the caller as `RecvError::Lagged(n)` — the SSE
    /// layer maps that to force-close.
    fn subscribe(&self, namespace: &str) -> broadcast::Receiver<Arc<Event>>;

    /// Prune events older than the given unix-epoch timestamp.
    /// Idempotent; callable from any node. Returns the number of rows
    /// deleted.
    async fn prune(&self, before_ts: f64) -> Result<u64>;

    /// Look up the oldest retained id for a namespace. Used by the SSE
    /// layer to decide 410 Gone when a client's `Last-Event-ID` is
    /// older than retention.
    async fn oldest_id(&self, namespace: &str) -> Result<Option<i64>>;
}
