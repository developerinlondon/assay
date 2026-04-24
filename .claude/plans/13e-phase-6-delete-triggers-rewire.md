# Plan 13e — Phase 6: Delete Triggers + Rewire Scheduler/Dispatch

> Parent plan: [13-v0.13.1-engine-events-outbox.md](13-v0.13.1-engine-events-outbox.md) Prev:
> [13d-phase-4-5-typed-wrapper-cutover.md](13d-phase-4-5-typed-wrapper-cutover.md) — Next:
> [13f-phase-7-sse-rewrite.md](13f-phase-7-sse-rewrite.md)

---

## Phase 6 — Remove trigger DDL + `subscribe_runnable`/`subscribe_tasks`; rewire scheduler

**This is the "no going back" phase.** After this commit, the Rust emit path is the only source of
dispatch wake-up signals. The prior phases must be green before this one lands.

**Files:**

- Modify: `crates/assay-workflow/src/store/postgres.rs` (delete `TRIGGER_DDL`, `subscribe_runnable`,
  `subscribe_tasks`, and the trigger `raw_sql` call)
- Modify: `crates/assay-workflow/src/store/sqlite.rs` (delete `subscribe_*` stubs)
- Modify: `crates/assay-domain/src/store/workflow.rs` (remove trait methods)
- Modify: `crates/assay-workflow/src/scheduler.rs` (new `run_dispatch_wakeups`, drop 15s scan)
- Modify: `crates/assay-workflow/src/dispatch_recovery.rs` (cadence 15s → 10min)
- Modify: `crates/assay-workflow/src/ctx.rs` (spawn `run_dispatch_wakeups` in `start`)

### Decision: existing `subscribe_trait_bounds` test

The workspace has `crates/assay-workflow/tests/subscribe_trait_bounds.rs` that compile-time asserts
the `subscribe_runnable` / `subscribe_tasks` shape. Delete this test file in this phase — the
methods are gone.

- [ ] **Step 6.1: Remove `TRIGGER_DDL` + related code in `postgres.rs`**

In `crates/assay-workflow/src/store/postgres.rs`:

- Delete the `TRIGGER_DDL` const (originally at lines 147-184).
- In `migrate()`, delete the line `sqlx::raw_sql(TRIGGER_DDL).execute(&self.pool).await?;` and its
  surrounding comments (originally around line 239).
- Delete the `impl WorkflowStore for PostgresStore` methods `subscribe_runnable` (line 1307) and
  `subscribe_tasks` (line 1350), plus any intervening helpers exclusively used by them.

Run after edit:

```bash
cargo check -p assay-workflow --features backend-postgres 2>&1 | tail -10
```

Expected: compile errors on the trait still having `subscribe_runnable` / `subscribe_tasks`. That's
the forcing function for step 6.3.

- [ ] **Step 6.2: Remove SQLite stubs**

In `crates/assay-workflow/src/store/sqlite.rs`, delete the `subscribe_runnable` and
`subscribe_tasks` stub impls from the `impl WorkflowStore for SqliteStore` block.

- [ ] **Step 6.3: Remove methods from the trait**

In `crates/assay-domain/src/store/workflow.rs` (around lines 420-460), delete the two trait method
declarations:

```rust
// DELETE:
//   fn subscribe_runnable<'a>(&'a self, namespace: &'a str)
//       -> impl futures_core::Stream<Item = String> + Send + 'a;
//   fn subscribe_tasks<'a>(&'a self, queue_names: &'a [&'a str])
//       -> impl futures_core::Stream<Item = String> + Send + 'a;
```

Also delete the accompanying doc-comments for those methods.

- [ ] **Step 6.4: Delete the `subscribe_trait_bounds` test**

```bash
git rm crates/assay-workflow/tests/subscribe_trait_bounds.rs
```

- [ ] **Step 6.5: Add `run_dispatch_wakeups` in `scheduler.rs`**

The existing `run_scheduler` (cron evaluator) stays unchanged. Add a _new_ function
`run_dispatch_wakeups` that consumes from the bus and acts on `WorkflowNeedsDispatch`:

```rust
// crates/assay-workflow/src/scheduler.rs

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, warn};

use assay_domain::events::EngineEventBus;

use crate::store::WorkflowStore;

/// Subscribe to engine_events and dispatch any workflow whose emit
/// indicates it's ready for work. One task per known namespace;
/// spawning is idempotent (the bus's PgListener registry dedups).
pub async fn run_dispatch_wakeups<S: WorkflowStore>(
    store: Arc<S>,
    bus: Arc<dyn EngineEventBus>,
) {
    let namespaces = match store.list_namespaces().await {
        Ok(v) => v,
        Err(e) => {
            warn!(?e, "dispatch-wakeups: list_namespaces failed; aborting");
            return;
        }
    };
    for ns in namespaces {
        let ns_name = ns.name.clone();
        let store = Arc::clone(&store);
        let bus = Arc::clone(&bus);
        tokio::spawn(async move {
            let mut rx = bus.subscribe(&ns_name);
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        // Filter: only act on workflow subsystem events
                        // that indicate dispatch readiness.
                        if ev.namespace != ns_name {
                            continue;
                        }
                        if ev.kind != "workflow_needs_dispatch" {
                            continue;
                        }
                        let Some(workflow_id) =
                            ev.payload.get("workflow_id").and_then(|v| v.as_str())
                        else {
                            continue;
                        };
                        if let Err(e) = dispatch_workflow(&*store, workflow_id).await {
                            warn!(?e, ns = %ns_name, %workflow_id, "dispatch failed");
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        warn!(lagged = n, ns = %ns_name, "dispatch-wakeup rx lagged");
                        // Idempotent: next event will re-wake us. Worst
                        // case dispatch_recovery's 10-min sweep catches
                        // anything we missed.
                    }
                    Err(RecvError::Closed) => {
                        debug!(ns = %ns_name, "dispatch-wakeup rx closed");
                        break;
                    }
                }
            }
        });
    }
}

/// Claim + dispatch one workflow. Placeholder — wire into the existing
/// dispatcher entrypoint; the pre-refactor `scheduler.rs` already had
/// a `dispatch_one` or equivalent that the old select-loop called on
/// notify. Route the same path here.
async fn dispatch_workflow<S: WorkflowStore>(
    store: &S,
    workflow_id: &str,
) -> anyhow::Result<()> {
    // TODO(phase 6): replace this stub with a call to the actual
    // dispatcher path the old subscribe_runnable loop used. The exact
    // function lives in scheduler.rs or tasks.rs depending on where
    // the dispatch claim was originally called; keep the signature
    // (`&dyn WorkflowStore, workflow_id: &str`) identical.
    let _ = (store, workflow_id);
    Ok(())
}
```

**Important:** the `dispatch_workflow` stub must be replaced with the actual dispatch logic used
previously by the `subscribe_runnable`-driven loop. Before deleting `subscribe_runnable`, locate the
handler the old `select!` called on `notify` (grep the 0.13.0 commit for the `listener.recv().await`
→ handler path) and lift it into `dispatch_workflow` verbatim.

- [ ] **Step 6.6: Spawn `run_dispatch_wakeups` in `WorkflowCtx::start`**

In `crates/assay-workflow/src/ctx.rs`, alongside the existing `_scheduler`, `_timer_poller`, etc.
spawns, add (after `bus` is wired via `with_event_bus`, since `start` doesn't have the bus yet, move
this spawn into a `start_with_bus` variant or perform the spawn in a dedicated method called after
`with_event_bus`):

```rust
pub fn spawn_dispatch_wakeups(&self) -> tokio::task::JoinHandle<()> {
    let Some(bus) = self.bus.as_ref() else {
        tracing::info!("no event bus wired; dispatch_wakeups not spawned");
        return tokio::spawn(async {});
    };
    tokio::spawn(crate::scheduler::run_dispatch_wakeups(
        Arc::clone(&self.store),
        bus.inner(),
    ))
}
```

Have `api::serve` call `state.spawn_dispatch_wakeups()` and store the join handle (or detach it —
background tasks are detached in the existing pattern).

- [ ] **Step 6.7: Remove the old `select! { notify | sleep(15s) scan }` in `scheduler.rs`**

If the old `subscribe_runnable`-driven loop lived in `scheduler.rs`, delete it. If it lived in
`dispatch_recovery.rs`, only delete the `subscribe_runnable` path — keep the `select!`'s periodic
scan branch but push its cadence to `Duration::from_secs(600)` (10 min, pure safety net).

Update `DISPATCH_RECOVERY_INTERVAL`:

```rust
const DISPATCH_RECOVERY_INTERVAL: Duration = Duration::from_secs(600);
```

- [ ] **Step 6.8: Update existing tests that referenced subscribe paths**

Search the workspace for tests that reference `subscribe_runnable` / `subscribe_tasks`:

```bash
grep -rn "subscribe_runnable\|subscribe_tasks" crates/assay-workflow/tests/ 2>&1
```

For each hit: either delete the test (if it purely covered the deleted surface) or rewrite it to
exercise the bus (if it covered scheduler wake-up behaviour). Tests worth rewriting to consume from
the bus:

- `crates/assay-workflow/tests/smoke_backends.rs` push-stream tests (if any): replace
  `subscribe_runnable(ns)` call with `bus.subscribe(ns)` + publish, filter by
  `workflow_needs_dispatch` kind.

- [ ] **Step 6.9: Verify + commit**

```bash
cargo check --workspace 2>&1 | tail -5
cargo test --workspace --lib --tests 2>&1 | tail -20
cargo test --test engine_smoke 2>&1 | tail -10
```

Expected: all green. The `engine_smoke` test exercises the HTTP surface and the new emit path should
continue to deliver events to SSE clients (still the old SSE handler — Phase 7 rewrites that).

```bash
git add crates/
git commit -m "$(cat <<'EOF'
refactor(workflow): delete trigger DDL + subscribe_* methods; scheduler uses bus

- postgres.rs: TRIGGER_DDL const, raw_sql(TRIGGER_DDL) call, and
  subscribe_runnable + subscribe_tasks impls all deleted.
- sqlite.rs: subscribe_* stubs deleted.
- workflow.rs trait: subscribe_runnable + subscribe_tasks methods removed.
- scheduler.rs: new run_dispatch_wakeups() subscribes per namespace,
  filters for workflow_needs_dispatch kind; calls dispatch_workflow
  (lifted from the old listener.recv handler). 15s polling scan is
  gone; legacy select! loop removed.
- dispatch_recovery.rs: cadence bumped from 15s → 10min — pure hygiene
  net, not correctness for NOTIFY dropouts.
- subscribe_trait_bounds test deleted; smoke tests updated to consume
  from bus.

After this commit the EngineEventBus is the *only* path for dispatch
wake-up. Cursor replay on reconnect covers connection blips; TCP
keepalive (phase 9) covers silent TCP death.
EOF
)"
```

---

## Exit criteria for Phase 6

```bash
cargo check --workspace                             # clean
cargo test --workspace --lib --tests                # all pass
cargo test --test engine_smoke                      # engine spawns + SSE delivers events
grep -rn "TRIGGER_DDL\|subscribe_runnable\|subscribe_tasks" crates/  # zero hits
git log --oneline -5                                # phases 1-6 commits
```

Move on to [13f-phase-7-sse-rewrite.md](13f-phase-7-sse-rewrite.md).
