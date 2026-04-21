use std::sync::Arc;

use tokio::task::JoinHandle;
use tracing::info;

use crate::dispatch_recovery;
use crate::health;
use crate::scheduler;
use crate::store::WorkflowStore;
use crate::timers;

pub mod activities;
pub mod children;
pub mod collections;
pub mod tasks;
pub mod workflows;

/// The workflow engine. Owns the store and manages background tasks
/// (scheduler, timer poller, health monitor).
///
/// The API layer holds an `Arc<WorkflowEngine<S>>` and delegates all operations here.
/// Event the engine broadcasts when a workflow transitions through
/// its lifecycle states. Subscribed to by the SSE stream (`/api/v1/
/// events/stream`) so the dashboard can refresh live instead of
/// requiring the operator to F5 after every action.
#[derive(Clone, Debug)]
pub struct EngineEvent {
    pub event_type: String,
    pub workflow_id: String,
    pub namespace: String,
}

pub struct WorkflowEngine<S: WorkflowStore> {
    pub(crate) store: Arc<S>,
    pub(crate) event_tx: Option<tokio::sync::broadcast::Sender<EngineEvent>>,
    _scheduler: JoinHandle<()>,
    _timer_poller: JoinHandle<()>,
    _health_monitor: JoinHandle<()>,
    _dispatch_recovery: JoinHandle<()>,
    #[cfg(feature = "s3-archival")]
    _archival: Option<JoinHandle<()>>,
}

impl<S: WorkflowStore> WorkflowEngine<S> {
    /// Start the engine with all background tasks.
    pub fn start(store: S) -> Self {
        let store = Arc::new(store);

        let _scheduler = tokio::spawn(scheduler::run_scheduler(Arc::clone(&store)));
        let _timer_poller = tokio::spawn(timers::run_timer_poller(Arc::clone(&store)));
        let _health_monitor = tokio::spawn(health::run_health_monitor(Arc::clone(&store)));
        let _dispatch_recovery = tokio::spawn(dispatch_recovery::run_dispatch_recovery(
            Arc::clone(&store),
        ));

        #[cfg(feature = "s3-archival")]
        let _archival = crate::archival::ArchivalConfig::from_env().map(|cfg| {
            tokio::spawn(crate::archival::run_archival(Arc::clone(&store), cfg))
        });

        info!("Workflow engine started");

        Self {
            store,
            event_tx: None,
            _scheduler,
            _timer_poller,
            _health_monitor,
            _dispatch_recovery,
            #[cfg(feature = "s3-archival")]
            _archival,
        }
    }

    /// Attach an event broadcaster. The API layer sets this up so the
    /// SSE stream (`/events/stream`) sees state transitions as they
    /// happen — powers the dashboard's live list refresh, no F5 loop.
    /// Returns the engine by value so callers can chain:
    ///
    /// ```ignore
    /// let engine = WorkflowEngine::start(store).with_event_broadcaster(tx);
    /// ```
    pub fn with_event_broadcaster(
        mut self,
        tx: tokio::sync::broadcast::Sender<EngineEvent>,
    ) -> Self {
        self.event_tx = Some(tx);
        self
    }

    /// Access the underlying store (for the API layer).
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Broadcast a state-transition event. No-op when no broadcaster
    /// is wired up (tests, embedders without an SSE surface). Errors
    /// from a channel with zero subscribers are silently dropped —
    /// that's the normal state between connections, not a failure.
    pub(crate) fn broadcast(&self, event_type: &str, workflow_id: &str, namespace: &str) {
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(EngineEvent {
                event_type: event_type.to_string(),
                workflow_id: workflow_id.to_string(),
                namespace: namespace.to_string(),
            });
        }
    }
}

/// Strip a trailing `-continued-<digits>` from a workflow id so
/// sequential continue-as-new calls don't pile up suffixes. Matches
/// the pattern emitted by the default id-derivation path; returns the
/// input unchanged if there's no such suffix.
pub(crate) fn strip_continued_suffix(id: &str) -> &str {
    if let Some(idx) = id.rfind("-continued-") {
        let (head, tail) = id.split_at(idx);
        // Only strip when the tail after "-continued-" is all digits.
        let rest = &tail["-continued-".len()..];
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            return head;
        }
    }
    id
}

pub(crate) fn timestamp_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// WorkflowEngine version (the binary version pulled from Cargo at build time).
/// Stamped into every workflow's search_attributes at start so operators
/// can correlate runs to the engine release that executed them.
pub(crate) const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Auto-stamp `assay_engine_version` into a workflow's search attributes.
/// Returns `Some` JSON string for the caller to store in the record.
///
/// If the caller already supplied `assay_engine_version` in their patch,
/// we leave their value alone (explicit override wins). Otherwise we
/// backfill the running engine's version. Callers who supply no
/// attributes at all get a single-key object with just the version.
pub(crate) fn inject_engine_version(caller_attrs: Option<&str>) -> Option<String> {
    let mut obj: serde_json::Map<String, serde_json::Value> = match caller_attrs {
        Some(raw) => match serde_json::from_str::<serde_json::Value>(raw) {
            Ok(serde_json::Value::Object(m)) => m,
            // Non-object JSON (or unparsable) — preserve as-is without
            // stamping; we can't safely merge a key into a non-object.
            Ok(other) => return Some(other.to_string()),
            Err(_) => return Some(raw.to_string()),
        },
        None => serde_json::Map::new(),
    };
    obj.entry("assay_engine_version".to_string())
        .or_insert_with(|| serde_json::Value::String(ENGINE_VERSION.to_string()));
    Some(serde_json::Value::Object(obj).to_string())
}

#[cfg(test)]
mod engine_version_stamp_tests {
    use super::*;

    #[test]
    fn no_attrs_produces_single_key_object() {
        let out = inject_engine_version(None).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["assay_engine_version"], ENGINE_VERSION);
        assert_eq!(v.as_object().unwrap().len(), 1);
    }

    #[test]
    fn existing_attrs_gain_the_version_field() {
        let out = inject_engine_version(Some(r#"{"env":"prod","tenant":"acme"}"#)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["env"], "prod");
        assert_eq!(v["tenant"], "acme");
        assert_eq!(v["assay_engine_version"], ENGINE_VERSION);
    }

    #[test]
    fn caller_supplied_version_wins_on_conflict() {
        let out = inject_engine_version(Some(r#"{"assay_engine_version":"0.0.1-test"}"#)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["assay_engine_version"], "0.0.1-test");
    }

    #[test]
    fn non_object_json_is_preserved_unchanged() {
        let out = inject_engine_version(Some("[1, 2, 3]")).unwrap();
        assert_eq!(out, "[1,2,3]");
    }

    #[test]
    fn unparsable_json_is_preserved_unchanged() {
        let out = inject_engine_version(Some("not json")).unwrap();
        assert_eq!(out, "not json");
    }
}
