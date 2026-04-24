# Plan 13d — Phase 4 + 5: `WorkflowEventBus` + Cutover

> Parent plan: [13-v0.13.1-engine-events-outbox.md](13-v0.13.1-engine-events-outbox.md) Prev:
> [13c-phase-3-backend-sqlite.md](13c-phase-3-backend-sqlite.md) — Next:
> [13e-phase-6-delete-triggers-rewire.md](13e-phase-6-delete-triggers-rewire.md)

---

## Phase 4 — `WorkflowEventBus` + `WorkflowEvent` enum

Typed workflow-event layer on top of the generic `EngineEventBus`.

**Files:**

- Modify: `crates/assay-workflow/Cargo.toml`
- Create: `crates/assay-workflow/src/events.rs`
- Modify: `crates/assay-workflow/src/lib.rs`

- [ ] **Step 4.1: Add feature pass-through in `assay-workflow/Cargo.toml`**

Under `[features]`:

```toml
default = ["backend-postgres", "backend-sqlite"]
backend-postgres = ["assay-domain/backend-postgres"]
backend-sqlite = ["assay-domain/backend-sqlite"]
```

Under `[dependencies]` (if not already present):

```toml
assay-domain = { path = "../assay-domain" }
```

Run:

```bash
cargo check -p assay-workflow
```

Expected: PASS.

- [ ] **Step 4.2: Define `WorkflowEvent` enum + `WorkflowEventBus`**

Create `crates/assay-workflow/src/events.rs`:

```rust
//! Typed workflow event layer on top of `assay_domain::events`.
//!
//! Every state-mutating method in `assay-workflow` that previously
//! called `ctx.broadcast(...)` or relied on a PG trigger for
//! `pg_notify` now emits a `WorkflowEvent` via `WorkflowEventBus`.

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
pub enum WorkflowEvent {
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

impl WorkflowEvent {
    /// The kind string written to `engine_events.kind`. Matches the
    /// serde `rename_all = "snake_case"` tag for each variant.
    pub fn kind(&self) -> &'static str {
        match self {
            WorkflowEvent::WorkflowCreated { .. } => "workflow_created",
            WorkflowEvent::WorkflowStatusChanged { .. } => "workflow_status_changed",
            WorkflowEvent::WorkflowNeedsDispatch { .. } => "workflow_needs_dispatch",
            WorkflowEvent::WorkflowStarted { .. } => "workflow_started",
            WorkflowEvent::WorkflowRunning { .. } => "workflow_running",
            WorkflowEvent::WorkflowCompleted { .. } => "workflow_completed",
            WorkflowEvent::WorkflowFailed { .. } => "workflow_failed",
            WorkflowEvent::WorkflowCancelled { .. } => "workflow_cancelled",
            WorkflowEvent::WorkflowTerminated { .. } => "workflow_terminated",
            WorkflowEvent::ActivityInserted { .. } => "activity_inserted",
            WorkflowEvent::ActivityStatusChanged { .. } => "activity_status_changed",
            WorkflowEvent::SignalReceived { .. } => "signal_received",
            WorkflowEvent::TimerFired { .. } => "timer_fired",
        }
    }

    /// Serialise the variant's fields to JSON for `engine_events.payload`.
    /// We strip the `kind` serde tag from the payload since the kind
    /// lives in its own column.
    pub fn payload(&self) -> serde_json::Value {
        let v = serde_json::to_value(self).expect("WorkflowEvent serialisable");
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
    pub async fn publish(&self, namespace: &str, ev: WorkflowEvent) -> Result<i64> {
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
        let e = WorkflowEvent::WorkflowCreated {
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
        let e = WorkflowEvent::ActivityInserted {
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
```

- [ ] **Step 4.3: Export from `assay-workflow/src/lib.rs`**

Add:

```rust
pub mod events;
```

- [ ] **Step 4.4: Verify + commit**

```bash
cargo test -p assay-workflow --lib events
```

Expected: both inline unit tests PASS.

```bash
git add crates/assay-workflow/src/events.rs crates/assay-workflow/src/lib.rs crates/assay-workflow/Cargo.toml
git commit -m "$(cat <<'EOF'
feat(workflow/events): WorkflowEvent enum + WorkflowEventBus wrapper

Typed variant per state transition with serde-driven kind tags.
Wraps Arc<dyn EngineEventBus> so subsystems share the underlying
table + channel + listener but produce/consume their own typed
events. Payload drops the kind tag since it lives in the column.

Unit tests: kind_tag_stripped_from_payload, activity_inserted_payload.
EOF
)"
```

---

## Phase 5 — Cutover `ctx.broadcast(...)` → typed `emit`

Rewire every call site that currently fires `self.broadcast(...)` to emit a typed event via the bus.
This phase does NOT yet touch the scheduler, SSE, or trigger DDL — those come in Phase 6/7. Here we
just flip the emit side so the bus starts seeing real traffic.

**Files:**

- Modify: `crates/assay-workflow/src/ctx.rs`
- Modify: `crates/assay-workflow/src/lifecycle.rs`
- Modify: `crates/assay-workflow/src/tasks.rs`
- Modify: `crates/assay-workflow/src/signals.rs`
- Modify: `crates/assay-workflow/src/timers.rs`
- Modify: `crates/assay-workflow/src/api/mod.rs`
- Modify: `crates/assay-engine/src/bin/assay-engine.rs`

### Call-site → variant mapping

| Existing call (string `kind`)                                | New variant                                                                      |
| ------------------------------------------------------------ | -------------------------------------------------------------------------------- |
| `broadcast("workflow_started", id, ns)`                      | `WorkflowEvent::WorkflowStarted { workflow_id: id.into() }`                      |
| `broadcast("workflow_running", id, ns)`                      | `WorkflowEvent::WorkflowRunning { workflow_id: id.into() }`                      |
| `broadcast("workflow_completed", id, ns)`                    | `WorkflowEvent::WorkflowCompleted { workflow_id: id.into() }`                    |
| `broadcast("workflow_failed", id, ns)`                       | `WorkflowEvent::WorkflowFailed { workflow_id: id.into() }`                       |
| `broadcast("workflow_cancelled", id, ns)`                    | `WorkflowEvent::WorkflowCancelled { workflow_id: id.into() }`                    |
| `broadcast("workflow_terminated", id, ns)`                   | `WorkflowEvent::WorkflowTerminated { workflow_id: id.into() }`                   |
| `broadcast("signal_received", id, ns)` + known `signal_name` | `WorkflowEvent::SignalReceived { workflow_id, signal_name }`                     |
| (new) where `needs_dispatch=true` is set in tx               | `WorkflowEvent::WorkflowNeedsDispatch { workflow_id, task_queue }`               |
| (new) on activity INSERT                                     | `WorkflowEvent::ActivityInserted { activity_id, workflow_id, task_queue, name }` |
| (new) on workflow_timers firing                              | `WorkflowEvent::TimerFired { workflow_id, seq }`                                 |

- [ ] **Step 5.1: Replace `event_tx`/`sse_tx` with `Option<WorkflowEventBus>` in `ctx.rs`**

Replace the relevant struct fields and methods in `crates/assay-workflow/src/ctx.rs`. Delete the old
`EngineEvent` and `BroadcastEvent` structs — SSE will consume `Event` (the engine-wide type)
directly. Delete `with_event_broadcaster`, `with_sse_tx`, and the old `broadcast(...)` method.

New shape:

```rust
use crate::events::{WorkflowEvent, WorkflowEventBus};

pub struct WorkflowCtx<S: WorkflowStore> {
    pub(crate) store: Arc<S>,
    pub(crate) bus: Option<WorkflowEventBus>,
    pub(crate) _bg: Arc<BackgroundTasks>,
    pub auth_mode: AuthMode,
    pub binary_version: Option<&'static str>,
}

impl<S: WorkflowStore> WorkflowCtx<S> {
    pub fn with_event_bus(mut self, bus: WorkflowEventBus) -> Self {
        self.bus = Some(bus);
        self
    }

    /// Expose the bus to subscribers (scheduler, SSE).
    pub fn bus(&self) -> Option<&WorkflowEventBus> {
        self.bus.as_ref()
    }

    /// Emit a typed workflow event. No-op when no bus is wired (tests,
    /// embedders without a dashboard). Errors are logged, not returned —
    /// an emission failure must not fail the state-mutating method that
    /// triggered it (atomicity for the state change is the DB tx's job;
    /// this is a notification we're firing *after* the row write).
    pub(crate) async fn emit(&self, namespace: &str, ev: WorkflowEvent) {
        if let Some(bus) = &self.bus {
            if let Err(e) = bus.publish(namespace, ev).await {
                tracing::warn!(?e, "engine event emit failed");
            }
        }
    }
}
```

Also drop the `EngineEvent` / `BroadcastEvent` struct defs and their re-exports.

- [ ] **Step 5.2: Replace call sites in `lifecycle.rs`**

For every call site `self.broadcast("workflow_X", workflow_id, namespace);`, replace with:

```rust
self.emit(namespace, WorkflowEvent::WorkflowX {
    workflow_id: workflow_id.to_string(),
}).await;
```

The mapping table above spells each one out. For `workflow_status_changed` (if the old code doesn't
emit this and just emits the specific lifecycle kind), introduce new `emit` calls at every place
that changes `workflow.status`:

```rust
self.emit(namespace, WorkflowEvent::WorkflowStatusChanged {
    workflow_id: id.to_string(),
    old_status: prev_status.to_string(),
    new_status: next_status.to_string(),
    task_queue: wf.task_queue.clone(),
}).await;
```

- [ ] **Step 5.3: Replace call sites + emit dispatch/activity events in `tasks.rs`**

Every call that transitions a workflow into `needs_dispatch = true` (used to trigger the PG
`assay_notify_runnable` trigger) now emits:

```rust
self.emit(namespace, WorkflowEvent::WorkflowNeedsDispatch {
    workflow_id: id.to_string(),
    task_queue: wf.task_queue.clone(),
}).await;
```

Place this emit immediately after the state-mutating store call returns. (In Phase 6 we'll confirm
the trigger is gone so this is the _only_ source of the signal.)

Every call that inserts a row into `workflow_activities` (old `assay_notify_task` trigger) now
emits:

```rust
self.emit(namespace, WorkflowEvent::ActivityInserted {
    activity_id,
    workflow_id: wf_id.to_string(),
    task_queue: task_queue.to_string(),
    name: activity_name.to_string(),
}).await;
```

And the existing `broadcast("workflow_running", id, ns)` in `tasks.rs:38` becomes the typed
`WorkflowEvent::WorkflowRunning { workflow_id }`.

- [ ] **Step 5.4: Replace call site in `signals.rs`**

`signals.rs:63` currently calls `self.broadcast("signal_received", workflow_id, &ns)` but doesn't
pass the signal name. Add it:

```rust
self.emit(&ns, WorkflowEvent::SignalReceived {
    workflow_id: workflow_id.to_string(),
    signal_name: signal_name.to_string(),
}).await;
```

- [ ] **Step 5.5: Emit `TimerFired` in `timers.rs`**

After a timer row's `fired = TRUE` is committed:

```rust
self.emit(&namespace, WorkflowEvent::TimerFired {
    workflow_id: wf_id.to_string(),
    seq,
}).await;
```

(Note: the current code may not have a broadcast for timers at all; this is a new signal type
surfaced by the refactor.)

- [ ] **Step 5.6: Update `api/mod.rs` wiring**

Replace the `sse_tx` + `engine_tx` + bridge-spawn block (~lines 95–125) with a single bus
construction. The bus is now passed in by the caller (assay-engine's main):

```rust
use std::sync::Arc;

use assay_domain::events::EngineEventBus;

use crate::events::WorkflowEventBus;

pub async fn serve<S: WorkflowStore + 'static>(
    store: impl Into<Arc<S>>,
    bus: Arc<dyn EngineEventBus>,
    port: u16,
    auth_mode: AuthMode,
    binary_version: Option<&'static str>,
) -> anyhow::Result<()> {
    let store = store.into();
    let wf_bus = WorkflowEventBus::new(bus);
    let mut ctx = WorkflowCtx::start(store)
        .with_event_bus(wf_bus)
        .with_auth_mode(auth_mode.clone());
    if let Some(v) = binary_version {
        ctx = ctx.with_binary_version(v);
    }
    let mode_desc = auth_mode.describe();
    let state = Arc::new(ctx);
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    info!("Workflow API listening on 0.0.0.0:{port} (auth: {mode_desc})");
    axum::serve(listener, app).await?;
    Ok(())
}
```

- [ ] **Step 5.7: Construct the bus in `assay-engine` main**

In `crates/assay-engine/src/bin/assay-engine.rs`, build the bus based on `cfg.backend` and pass it
into `serve`. Example:

```rust
use std::sync::Arc;

use assay_domain::events::EngineEventBus;

let bus: Arc<dyn EngineEventBus> = match cfg.backend {
    Backend::Postgres => {
        let pool = /* existing PG pool */;
        let url = /* the URL used for the pool */;
        Arc::new(
            assay_domain::events::PgEngineEventBus::new(pool.clone(), &url)
                .await?,
        )
    }
    Backend::Sqlite => {
        let pool = /* existing SQLite pool */;
        Arc::new(
            assay_domain::events::SqliteEngineEventBus::new(pool.clone())
                .await?,
        )
    }
};
assay_workflow::api::serve(store, bus, cfg.port, auth_mode, binary_version).await
```

- [ ] **Step 5.8: Verify + commit**

```bash
cargo check --workspace 2>&1 | tail -5
cargo test --workspace --lib --tests 2>&1 | tail -20
```

Expected: PASS. Existing tests continue to pass because the legacy trigger-based scheduler wake-up
is still live (Phase 6 removes it). The new emit path runs in parallel — we're not yet consuming
from the bus in production code, just exercising it via the new emits.

```bash
git add crates/
git commit -m "$(cat <<'EOF'
feat(workflow): emit typed WorkflowEvent via EngineEventBus at call sites

Replaces ctx.broadcast(kind_str, ...) with self.emit(ns, WorkflowEvent::X {..}).
Cutover is per call site in lifecycle.rs, tasks.rs, signals.rs, timers.rs;
new emits for WorkflowNeedsDispatch and ActivityInserted previously carried
by PG triggers are added at the store-method call sites.

api/mod.rs wiring now takes an Arc<dyn EngineEventBus> argument; assay-engine
main() selects the concrete backend bus based on EngineConfig.backend.

Legacy EngineEvent / BroadcastEvent types + sse_tx / engine_tx channels
are removed from ctx.rs. Trigger DDL and subscribe_* impls still in place
for backwards-compatible behaviour during this phase — removed in phase 6.
EOF
)"
```

---

## Exit criteria for Phase 5

```bash
cargo check --workspace                             # clean
cargo test --workspace --lib --tests                # all pre-existing tests PASS
git log --oneline -4                                # shows phases 1-5 commits
```

Move on to [13e-phase-6-delete-triggers-rewire.md](13e-phase-6-delete-triggers-rewire.md).
