# Plan 13a — Phase 0 + 1: Branch Setup & `EngineEventBus` Trait

> Parent plan: [13-v0.13.1-engine-events-outbox.md](13-v0.13.1-engine-events-outbox.md) Next:
> [13b-phase-2-backend-pg.md](13b-phase-2-backend-pg.md)

---

## Phase 0 — Branch setup + baseline verification

Confirm we're starting from a clean v0.13.0-shipped state so any regression is attributable.

- [ ] **Step 0.1: Verify branch + baseline build**

```bash
git status                 # clean, on feature/0.13.1-engine-events-outbox
cargo check --workspace    # clean
cargo test --workspace --lib --tests 2>&1 | tail -20   # PASS
```

Expected: clean checkout, clean check, clean tests. If anything fails, STOP — the bug pre-exists and
must be filed/fixed before this refactor.

- [ ] **Step 0.2: Plan file already committed**

The parent plan and phase files (13, 13a–13g) are on this branch already. Move on to Phase 1.

---

## Phase 1 — `EngineEventBus` trait + base types

No impl yet; we lock the trait shape first so both backends implement the same surface and
`assay-workflow` can start consuming it.

**Files:**

- Create: `crates/assay-domain/src/events/mod.rs`
- Create: `crates/assay-domain/src/events/trait_.rs`
- Modify: `crates/assay-domain/src/lib.rs`
- Modify: `crates/assay-domain/Cargo.toml`
- Create: `crates/assay-domain/tests/event_bus_trait_shape.rs`

- [ ] **Step 1.1: Add deps to `assay-domain/Cargo.toml`**

Add under `[dependencies]`:

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
futures-util = "0.3"
futures-core = "0.3"
async-trait = "0.1"
anyhow = "1"
thiserror = "2"
tokio = { version = "1", features = ["sync"] }
tracing = "0.1"
```

Do not add `sqlx` here yet — backend impls are feature-gated in Phase 2/3.

Run:

```bash
cargo check -p assay-domain
```

Expected: PASS.

- [ ] **Step 1.2: Create `events/mod.rs`**

```rust
//! Engine-wide CDC outbox. Every state-mutating store method writes a
//! typed event via a subsystem wrapper (e.g. `WorkflowEventBus`) that
//! in turn calls [`EngineEventBus::publish_committed`]. Subscribers —
//! scheduler, task workers, SSE dashboards — consume from a node-local
//! `tokio::broadcast` fed by same-node writes plus a single PG `LISTEN`
//! bridge for cross-node bumps. Events are durable in the `engine_events`
//! table with a configurable TTL (default 3 days) so reconnecting
//! clients can replay from a cursor (`Last-Event-ID` in SSE, `after:
//! Option<i64>` on the bus).

pub mod trait_;
pub use trait_::*;
```

- [ ] **Step 1.3: Define trait + types in `events/trait_.rs`**

```rust
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
            // Unknown / forward-rolled peer nodes: treat as `System` to
            // avoid panics. Logged by callers.
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
```

- [ ] **Step 1.4: Wire up `assay-domain/src/lib.rs`**

Add at the top of the module body (if there's already an existing `pub mod store;` or similar, put
this alongside):

```rust
pub mod events;
```

- [ ] **Step 1.5: Write compile-time trait shape test**

Create `crates/assay-domain/tests/event_bus_trait_shape.rs`:

```rust
//! Compile-time check that the trait surface is what we expect —
//! lets future refactors flag accidental API breaks at `cargo test`
//! time rather than by downstream crates failing to build.

use std::sync::Arc;

use assay_domain::events::{EngineEventBus, Event, EventFilter, NewEvent, Subsystem};

fn _is_dyn_compatible() {
    // `EngineEventBus` uses `#[async_trait]` which boxes futures, so
    // it IS dyn-compatible. Downstream crates rely on
    // `Arc<dyn EngineEventBus>`. This function must compile.
    let _: Option<Arc<dyn EngineEventBus>> = None;
}

fn _filter_compiles() {
    let f = EventFilter {
        subsystems: vec![Subsystem::Workflow],
        kinds: vec!["workflow_created".to_string()],
        workflow_id: Some("wf-1".to_string()),
    };
    let e = Event {
        id: 1,
        ts: 0.0,
        namespace: "main".into(),
        subsystem: Subsystem::Workflow,
        kind: "workflow_created".into(),
        payload: serde_json::json!({ "workflow_id": "wf-1" }),
    };
    assert!(f.matches(&e));
}

fn _new_event_compiles() {
    let _ = NewEvent {
        namespace: "main",
        subsystem: Subsystem::Workflow,
        kind: "workflow_created",
        payload: serde_json::json!({}),
    };
}

fn _subsystem_round_trip() {
    assert_eq!(Subsystem::from_str(Subsystem::Workflow.as_str()), Subsystem::Workflow);
    assert_eq!(Subsystem::from_str(Subsystem::Auth.as_str()), Subsystem::Auth);
    assert_eq!(Subsystem::from_str(Subsystem::Secrets.as_str()), Subsystem::Secrets);
    assert_eq!(Subsystem::from_str(Subsystem::System.as_str()), Subsystem::System);
    // Unknown maps to System (forward-compat)
    assert_eq!(Subsystem::from_str("unknown_subsystem"), Subsystem::System);
}

#[test]
fn shapes_hold() {
    _is_dyn_compatible();
    _filter_compiles();
    _new_event_compiles();
    _subsystem_round_trip();
}
```

- [ ] **Step 1.6: Verify + commit**

Run:

```bash
cargo test -p assay-domain --test event_bus_trait_shape
```

Expected: PASS (compiles + the single test passes). Then:

```bash
cargo check --workspace 2>&1 | tail -3
```

Expected: clean. Then commit:

```bash
git add crates/assay-domain/
git commit -m "$(cat <<'EOF'
feat(domain/events): EngineEventBus trait + Event/NewEvent/Subsystem types

Locks the trait surface and common types used by PG + SQLite impls
(coming next) and by per-subsystem typed wrappers. No backend impl
yet; compile-time shape test passes.

Types:
- Subsystem: Workflow | Auth | Secrets | System (snake_case)
- Event: id, ts, namespace, subsystem, kind, payload
- NewEvent<'a>: inputs for publish_committed
- EventFilter: subsystems / kinds / workflow_id (server-side filter)
- CursorGoneError: SSE 410 Gone signal

Trait is #[async_trait] (box futures) so Arc<dyn EngineEventBus> works.
EOF
)"
```

---

## Exit criteria for Phase 1

```bash
cargo test -p assay-domain --test event_bus_trait_shape  # PASS
cargo check --workspace                                   # clean
git log --oneline -1                                      # shows the commit
```

Move on to [13b-phase-2-backend-pg.md](13b-phase-2-backend-pg.md).
