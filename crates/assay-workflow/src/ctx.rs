use std::sync::Arc;

use tokio::task::JoinHandle;
use tracing::info;

use crate::auth_mode::AuthMode;
use crate::dispatch_recovery;
use crate::events::{WorkflowBusEvent, WorkflowEventBus};
use crate::health;
use crate::scheduler;
use crate::store::WorkflowStore;
use crate::timers;

/// Holds the background-task JoinHandles. When the last Arc<BackgroundTasks>
/// is dropped the tasks are abandoned (tokio will cancel them on shutdown).
pub struct BackgroundTasks {
    _scheduler: JoinHandle<()>,
    _timer_poller: JoinHandle<()>,
    _health_monitor: JoinHandle<()>,
    _dispatch_recovery: JoinHandle<()>,
    #[cfg(feature = "s3-archival")]
    _archival: Option<JoinHandle<()>>,
}

/// The workflow context. Owns the store, background-task handles, and
/// per-request config. Serves as both the orchestrator (all engine methods
/// live as `impl WorkflowCtx<S>`) and the axum state (`Arc<WorkflowCtx<S>>`).
///
/// `S` is the concrete store backend (`SqliteStore` or `PostgresStore`).
/// `WorkflowStore` uses RPIT futures and is not dyn-compatible, so the
/// generic parameter is retained here to avoid boxing every async call.
pub struct WorkflowCtx<S: WorkflowStore> {
    pub(crate) store: Arc<S>,
    /// Engine-wide CDC outbox. When wired, state-mutating methods
    /// publish typed `WorkflowBusEvent` variants via `emit(...)`.
    /// `None` for tests / embedders without a dashboard — emit becomes
    /// a no-op.
    pub(crate) bus: Option<WorkflowEventBus>,
    pub(crate) _bg: Arc<BackgroundTasks>,
    pub auth_mode: AuthMode,
    /// Version of the containing binary (e.g. the `assay-lua` CLI) — set
    /// by embedders so `/api/v1/engine/workflow/version` reflects the user-facing
    /// binary, not this internal engine-crate version.
    pub binary_version: Option<&'static str>,
}

impl<S: WorkflowStore> WorkflowCtx<S> {
    /// Start the context with all background tasks.
    pub fn start(store: Arc<S>) -> Self {
        let _scheduler = tokio::spawn(scheduler::run_scheduler(Arc::clone(&store)));
        let _timer_poller = tokio::spawn(timers::run_timer_poller(Arc::clone(&store)));
        let _health_monitor = tokio::spawn(health::run_health_monitor(Arc::clone(&store)));
        let _dispatch_recovery = tokio::spawn(dispatch_recovery::run_dispatch_recovery(
            Arc::clone(&store),
        ));

        #[cfg(feature = "s3-archival")]
        let _archival = crate::archival::ArchivalConfig::from_env()
            .map(|cfg| tokio::spawn(crate::archival::run_archival(Arc::clone(&store), cfg)));

        info!("Workflow engine started");

        Self {
            store,
            bus: None,
            _bg: Arc::new(BackgroundTasks {
                _scheduler,
                _timer_poller,
                _health_monitor,
                _dispatch_recovery,
                #[cfg(feature = "s3-archival")]
                _archival,
            }),
            auth_mode: AuthMode::default(),
            binary_version: None,
        }
    }

    /// Attach the engine-wide event bus. The API layer sets this up so
    /// the SSE stream (`/events/stream`) and the dispatch-wakeup loop
    /// see state transitions as they happen. Returns the context by
    /// value so callers can chain.
    pub fn with_event_bus(mut self, bus: WorkflowEventBus) -> Self {
        self.bus = Some(bus);
        self
    }

    /// Set the auth mode.
    pub fn with_auth_mode(mut self, auth_mode: AuthMode) -> Self {
        self.auth_mode = auth_mode;
        self
    }

    /// Set the binary version string surfaced by `/api/v1/engine/workflow/version`.
    pub fn with_binary_version(mut self, version: &'static str) -> Self {
        self.binary_version = Some(version);
        self
    }

    /// Access the underlying store (for the API layer).
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Access the event bus (for SSE + scheduler wake-up).
    pub fn bus(&self) -> Option<&WorkflowEventBus> {
        self.bus.as_ref()
    }

    /// Emit a typed workflow event. No-op when no bus is wired (tests,
    /// embedders without a dashboard). Errors are logged, not returned —
    /// an emission failure must not fail the state-mutating method that
    /// triggered it (atomicity for the state change is the DB tx's job;
    /// this is a notification fired *after* the row write).
    pub(crate) async fn emit(&self, namespace: &str, ev: WorkflowBusEvent) {
        if let Some(bus) = &self.bus
            && let Err(e) = bus.publish(namespace, ev).await
        {
            tracing::warn!(?e, "engine event emit failed");
        }
    }

    /// Mark a workflow dispatchable AND emit a `WorkflowNeedsDispatch`
    /// on the bus so the dispatch-wakeup loop (phase 6) wakes workers
    /// on this node / across the cluster. The extra SELECT is skipped
    /// when no bus is wired.
    pub(crate) async fn mark_and_emit_needs_dispatch(
        &self,
        workflow_id: &str,
    ) -> anyhow::Result<()> {
        self.store.mark_workflow_dispatchable(workflow_id).await?;
        if self.bus.is_some()
            && let Some(wf) = self.store.get_workflow(workflow_id).await?
        {
            self.emit(
                &wf.namespace,
                WorkflowBusEvent::WorkflowNeedsDispatch {
                    workflow_id: workflow_id.to_string(),
                    task_queue: wf.task_queue,
                },
            )
            .await;
        }
        Ok(())
    }
}

/// Strip a trailing `-continued-<digits>` from a workflow id so
/// sequential continue-as-new calls don't pile up suffixes. Matches
/// the pattern emitted by the default id-derivation path; returns the
/// input unchanged if there's no such suffix.
pub(crate) fn strip_continued_suffix(id: &str) -> &str {
    if let Some(idx) = id.rfind("-continued-") {
        let (head, tail) = id.split_at(idx);
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

/// WorkflowCtx version (the binary version pulled from Cargo at build time).
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
        let out =
            inject_engine_version(Some(r#"{"assay_engine_version":"0.0.1-test"}"#)).unwrap();
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
