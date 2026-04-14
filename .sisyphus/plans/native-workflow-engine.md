# Plan: Native Durable Workflow Engine for Assay

## Summary

Replace the Temporal server dependency with a self-contained, SQLite-backed durable workflow engine that runs entirely in-process. Assay already implements Temporal's coroutine-based workflow execution model (temporal_worker.rs, 1158 lines) — this plan adds the missing persistence layer (event history store + state machine) so workflows survive crashes, restarts, and run without any external server.

## Motivation

**Today**: assay depends on a running Temporal cluster to:
- Persist workflow event history
- Dispatch workflow tasks to workers
- Manage timers, signals, and queries
- Handle retries and timeouts

**Problem**: Deploying Temporal (Cassandra/Postgres + server + matching + frontend + worker) defeats assay's value proposition — a single 9MB binary that replaces 250MB Python/Node containers.

**Goal**: assay becomes a self-contained workflow platform. Write durable workflows in Lua, backed by SQLite. No external dependencies.

## What Already Exists (70% of the hard parts)

The following are already built and production-quality in assay:

| Component | File | What It Does |
|---|---|---|
| Coroutine workflow model | `temporal_worker.rs:82-170` | `CTX_LUA` — deterministic ctx with `execute_activity`, `wait_signal`, `sleep`, `side_effect`, `register_query` |
| Command yield/resume | `temporal_worker.rs:931-1144` | `process_coroutine_result` — parses yielded command tables into typed workflow commands |
| Replay buffers | `temporal_worker.rs:470-530` | Resolved activities, fired timers, signal buffers — all correctly populated during replay |
| Activity dispatch | `temporal_worker.rs:260-376` | Poll → Lua function call → complete with result/error |
| Signal handling | `temporal_worker.rs:1077-1094` | `wait_signal` with optional timeout timer + signal buffering |
| Timer management | `temporal_worker.rs:1100-1111` | `ctx:sleep` → StartTimer command |
| Query handlers | `temporal_worker.rs:745-801` | `ctx:register_query` + dispatch from activation |
| PendingWait state machine | `temporal_worker.rs:1148-1153` | Tracks what each coroutine is waiting for (Activity, Signal, Timer) |
| SQLite support | `db.rs` (via sqlx) | `db.connect/query/execute/close` — SQLite, Postgres, MySQL |
| Async runtime | `core.rs:448-535` | `async.spawn`, `async.spawn_interval` with tokio + LocalSet |

## Architecture

```
+====================================================================+
|  ASSAY (single process, ~9MB binary)                               |
|                                                                    |
|  +--------------------------------------------------------------+  |
|  | Lua VM                                                        |  |
|  |   workflow.define("DeployService", function(ctx, input)       |  |
|  |     local result = ctx:execute_activity("provision_k8s", {...|
|  |     ctx:sleep(30)   -- durable timer                          |  |
|  |     local signal = ctx:wait_signal("approval")                |  |
|  |     ctx:execute_activity("deploy", { ... })                   |  |
|  |     return { status = "deployed" }                            |  |
|  |   end)                                                        |  |
|  +--------------------------------------------------------------+  |
|          | yields commands                                          |
|          v                                                          |
|  +--------------------------------------------------------------+  |
|  | Rust Workflow Engine                                          |  |
|  |   +-------------------+    +----------------------------+     |  |
|  |   | Workflow State    |    | Activity Executor          |     |  |
|  |   | Machine           |    |  (tokio tasks, retriable)  |     |  |
|  |   | - Running         |    +----------------------------+     |  |
|  |   | - Waiting         |                                       |  |
|  |   | - Completed       |    +----------------------------+     |  |
|  |   | - Failed          |    | Timer Manager              |     |  |
|  |   | - Cancelled       |    |  (tokio::time, persistent) |     |  |
|  |   +-------------------+    +----------------------------+     |  |
|  |          |                          |                          |  |
|  |          v                          v                          |  |
|  |   +--------------------------------------------------------+  |  |
|  |   | Event Store (SQLite WAL mode)                          |  |  |
|  |   |                                                        |  |  |
|  |   | workflows: id, type, status, run_id, created_at        |  |  |
|  |   | events:    seq, type, payload, timestamp                |  |  |
|  |   | timers:    seq, fire_at, workflow_id                    |  |  |
|  |   | signals:   name, payload, received_at, workflow_id      |  |  |
|  |   | snapshots: event_seq, state_json (checkpoint)           |  |  |
|  |   +--------------------------------------------------------+  |  |
|  +--------------------------------------------------------------+  |
+====================================================================+
```

## SQLite Schema

```sql
-- Core workflow state
CREATE TABLE IF NOT EXISTS workflows (
    id              TEXT PRIMARY KEY,           -- workflow_id
    run_id          TEXT NOT NULL,              -- current run
    workflow_type   TEXT NOT NULL,              -- "DeployService"
    task_queue      TEXT NOT NULL DEFAULT 'default',
    status          TEXT NOT NULL DEFAULT 'RUNNING',
                                 -- RUNNING | WAITING | COMPLETED | FAILED | CANCELLED | TIMED_OUT
    input           TEXT,                       -- JSON payload
    result          TEXT,                       -- JSON result (when completed)
    error           TEXT,                       -- error message (when failed)
    parent_id       TEXT,                       -- parent workflow_id (for child workflows)
    created_at      REAL NOT NULL,              -- unix timestamp
    updated_at      REAL NOT NULL,
    completed_at    REAL
);

-- Append-only event history (event sourcing)
CREATE TABLE IF NOT EXISTS events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    workflow_id     TEXT NOT NULL REFERENCES workflows(id),
    seq             INTEGER NOT NULL,           -- monotonic within workflow
    event_type      TEXT NOT NULL,              -- see event types below
    payload         TEXT,                       -- JSON event data
    timestamp       REAL NOT NULL,
    FOREIGN KEY (workflow_id) REFERENCES workflows(id)
);

CREATE INDEX idx_events_workflow_seq ON events(workflow_id, seq);

-- Event types:
--   WorkflowStarted        { input, workflow_type }
--   ActivityScheduled      { seq, name, input, timeout_opts }
--   ActivityCompleted      { seq, result }
--   ActivityFailed         { seq, error }
--   TimerStarted           { seq, duration }
--   TimerFired             { seq }
--   SignalReceived         { name, payload }
--   WorkflowCompleted      { result }
--   WorkflowFailed         { error }
--   WorkflowCancelled      { reason }

-- Pending timers (for crash recovery)
CREATE TABLE IF NOT EXISTS timers (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    workflow_id     TEXT NOT NULL,
    seq             INTEGER NOT NULL,
    fire_at         REAL NOT NULL,              -- unix timestamp
    fired           INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (workflow_id) REFERENCES workflows(id)
);

CREATE INDEX idx_timers_fire ON timers(fire_at) WHERE fired = 0;

-- Pending signals (buffered until workflow consumes them)
CREATE TABLE IF NOT EXISTS signals (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    workflow_id     TEXT NOT NULL,
    name            TEXT NOT NULL,
    payload         TEXT,                       -- JSON
    consumed        INTEGER NOT NULL DEFAULT 0,
    received_at     REAL NOT NULL,
    FOREIGN KEY (workflow_id) REFERENCES workflows(id)
);

CREATE INDEX idx_signals_workflow ON signals(workflow_id, name, consumed);

-- Snapshots (periodic state checkpoints for fast replay)
CREATE TABLE IF NOT EXISTS snapshots (
    workflow_id     TEXT NOT NULL,
    event_seq       INTEGER NOT NULL,           -- snapshot taken after this event
    state_json      TEXT NOT NULL,              -- serialized coroutine + ctx state
    created_at      REAL NOT NULL,
    PRIMARY KEY (workflow_id, event_seq)
);

-- Activity lock table (for distributed-like locking in single-process)
CREATE TABLE IF NOT EXISTS activities (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    workflow_id     TEXT NOT NULL,
    seq             INTEGER NOT NULL,
    name            TEXT NOT NULL,
    input           TEXT,                       -- JSON
    status          TEXT NOT NULL DEFAULT 'PENDING',
                                 -- PENDING | RUNNING | COMPLETED | FAILED | CANCELLED
    result          TEXT,
    error           TEXT,
    attempt         INTEGER NOT NULL DEFAULT 1,
    max_attempts    INTEGER NOT NULL DEFAULT 3,
    timeout_secs    REAL NOT NULL DEFAULT 300,
    scheduled_at    REAL NOT NULL,
    started_at      REAL,
    completed_at    REAL,
    FOREIGN KEY (workflow_id) REFERENCES workflows(id)
);

CREATE INDEX idx_activities_pending ON activities(status, scheduled_at) WHERE status = 'PENDING';
```

## API Design (Lua)

### workflow.define — Register a workflow

```lua
workflow.define("DeployService", function(ctx, input)
    log.info("deploying " .. input.service_name)

    -- Durable activity execution (survives crashes, retried on failure)
    local provision = ctx:execute_activity("provision_k8s", {
        namespace = input.namespace,
        manifest = input.manifest,
    }, {
        timeout = 300,         -- seconds
        retry = { max_attempts = 3, backoff = 5 },
    })

    -- Durable sleep (survives crashes — fires from SQLite timer table)
    ctx:sleep(30)

    -- Wait for external signal (human approval, webhook, etc.)
    local approval = ctx:wait_signal("approval", { timeout = 3600 })
    if not approval then
        error("timed out waiting for approval")
    end

    -- Another durable activity
    local deploy = ctx:execute_activity("deploy", {
        namespace = input.namespace,
        image = input.image,
    })

    -- Register a query handler (read-only state inspection)
    ctx:register_query("status", function()
        return { phase = "deployed", image = input.image }
    end)

    return { status = "deployed", provision_id = provision.id }
end)
```

### workflow.activity — Register an activity

```lua
workflow.activity("provision_k8s", function(input)
    local k8s = require("assay.k8s")
    local c = k8s.client(env.get("K8S_URL"), { token = env.get("K8S_TOKEN") })
    return c:apply(input.namespace, input.manifest)
end)
```

### workflow.start — Start a workflow

```lua
local handle = workflow.start("DeployService", {
    workflow_id = "deploy-svc-" .. os.hostname(),
    input = {
        service_name = "my-app",
        namespace = "production",
        image = "ghcr.io/org/app:v1.2.3",
        manifest = fs.read("manifests/app.yaml"),
    },
})
log.info("started workflow: " .. handle.workflow_id)
```

### workflow.signal — Send a signal

```lua
workflow.signal("deploy-svc-prod-01", "approval", { approved = true, by = "alice" })
```

### workflow.query — Query workflow state

```lua
local status = workflow.query("deploy-svc-prod-01", "status")
log.info(json.encode(status))  -- { phase = "deployed", image = "..." }
```

### workflow.describe — Get workflow status

```lua
local info = workflow.describe("deploy-svc-prod-01")
-- { status = "RUNNING", started_at = 1712985600, ... }
```

### workflow.cancel / workflow.terminate

```lua
workflow.cancel("deploy-svc-prod-01")
workflow.terminate("deploy-svc-prod-01", "deployment rolled back")
```

### workflow.list — List workflows

```lua
local workflows = workflow.list({ status = "RUNNING", limit = 20 })
for _, wf in ipairs(workflows) do
    log.info(wf.id .. " " .. wf.status)
end
```

### workflow.run — Start engine event loop

```lua
-- Register everything, then start the engine
workflow.define("DeployService", deploy_fn)
workflow.activity("provision_k8s", provision_fn)
workflow.activity("deploy", deploy_fn)

-- Start the engine (blocks until shutdown)
workflow.run({
    db = "sqlite:///var/lib/assay/workflows.db",
    task_queue = "default",
    shutdown_on_idle = false,   -- keep running (server mode)
})
```

## Implementation Phases

### Phase 1: Core Engine (MVP) — ~800-1000 lines Rust

**Goal**: Durable workflow execution with SQLite persistence. Activities, timers, and basic error handling.

| Step | Description | Files |
|---|---|---|
| 1.1 | SQLite schema initialization + migrations | `src/workflow/schema.rs` |
| 1.2 | Event store (append, read, replay) | `src/workflow/store.rs` |
| 1.3 | Workflow state machine (status transitions) | `src/workflow/state.rs` |
| 1.4 | Coroutine dispatch loop (adapted from temporal_worker.rs) | `src/workflow/engine.rs` |
| 1.5 | Activity executor (tokio tasks + retry) | `src/workflow/activities.rs` |
| 1.6 | Timer manager (persistent timers from SQLite) | `src/workflow/timers.rs` |
| 1.7 | Lua bindings (`workflow.*` API) | `src/lua/builtins/workflow.rs` |
| 1.8 | Feature flag (`workflow` feature, default on) | `Cargo.toml` |

**Delivers**:
- `workflow.define()` / `workflow.activity()` / `workflow.start()`
- `ctx:execute_activity()` / `ctx:sleep()`
- SQLite-backed event history
- Crash recovery (replay from event log)
- Activity retries with exponential backoff

### Phase 2: Signals & Queries — ~400 lines Rust

**Goal**: External event handling and state inspection.

| Step | Description | Files |
|---|---|---|
| 2.1 | Signal buffering in SQLite | `src/workflow/signals.rs` |
| 2.2 | `ctx:wait_signal()` with timeout | `src/workflow/engine.rs` |
| 2.3 | `workflow.signal()` API | `src/lua/builtins/workflow.rs` |
| 2.4 | `ctx:register_query()` + `workflow.query()` | `src/workflow/queries.rs` |
| 2.5 | `workflow.describe()` / `workflow.list()` | `src/lua/builtins/workflow.rs` |

**Delivers**:
- External signals (webhook → `workflow.signal()`)
- Signal buffering across workflow pauses
- Query handlers (read-only state inspection)
- Workflow listing and status queries

### Phase 3: Child Workflows & Cancellation — ~300 lines Rust

**Goal**: Nested workflows and graceful shutdown.

| Step | Description | Files |
|---|---|---|
| 3.1 | `ctx:execute_child_workflow()` | `src/workflow/engine.rs` |
| 3.2 | Cancellation propagation (parent → child) | `src/workflow/cancel.rs` |
| 3.3 | `workflow.cancel()` / `workflow.terminate()` | `src/lua/builtins/workflow.rs` |
| 3.4 | `ctx:side_effect()` (non-deterministic operations) | `src/workflow/engine.rs` |

**Delivers**:
- Nested workflow execution
- Cancellation propagation
- Non-deterministic side effects

### Phase 4: Durability & Operations — ~300 lines Rust

**Goal**: Production reliability features.

| Step | Description | Files |
|---|---|---|
| 4.1 | Periodic snapshots (fast replay after N events) | `src/workflow/snapshot.rs` |
| 4.2 | Event compaction (truncate history after snapshot) | `src/workflow/store.rs` |
| 4.3 | Continue-as-new (workflow restart with state) | `src/workflow/engine.rs` |
| 4.4 | CLI commands: `assay workflow list`, `assay workflow signal`, etc. | `src/main.rs` |
| 4.5 | Migration from temporal feature (keep backward compat) | `Cargo.toml` |

**Delivers**:
- Fast crash recovery via snapshots
- Long-running workflow support (continue-as-new)
- CLI management tools
- Clean migration path from Temporal

### Phase 5: Advanced Features (Future) — Optional

- **Schedules**: Cron-like recurring workflows (`workflow.schedule()`)
- **Search attributes**: Tag workflows with metadata for filtering
- **Versioning**: Workflow code changes without breaking running instances
- **Observability**: Prometheus metrics, structured logging
- **Multi-process**: Optional network mode for distributed workers (gRPC bridge)

## Key Design Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Storage | SQLite (via sqlx) | Already a dependency. WAL mode for durability. No external services. |
| Execution model | Lua coroutines (reuse CTX_LUA) | Already battle-tested in temporal_worker.rs. Deterministic replay works. |
| Concurrency | Single-process, tokio tasks | No distributed coordination needed. Simpler, faster, matches assay's use case. |
| Event format | JSON in SQLite TEXT columns | Assay already speaks JSON natively. Human-readable, debuggable. |
| Timer persistence | SQLite timers table + tokio::time | Timers survive crashes (replayed from DB on startup). |
| Replay strategy | Event replay + periodic snapshots | Pure replay is O(n) on history length. Snapshots cap replay time. |
| Feature flag | `workflow` feature (default on) | Keeps temporal feature for backward compat. Clean separation. |
| API surface | `workflow.*` global (new namespace) | Doesn't conflict with existing `temporal.*`. Clean migration path. |

## What Gets Removed (Eventually)

When the native engine is stable, the `temporal` feature flag and its dependencies can be deprecated:

```
temporalio-client    ~0.2.0   (optional, can remove)
temporalio-sdk       ~0.2.0   (optional, can remove)
temporalio-sdk-core  ~0.2.0   (optional, can remove)
temporalio-common    ~0.2.0   (optional, can remove)
prost-wkt-types      ~0.7     (only used by temporal)
```

This removes ~5 dependency crates and their transitive deps (protobuf, gRPC, etc.), potentially reducing binary size by 1-2MB.

## File Structure

```
src/
├── workflow/                    # NEW: Native workflow engine
│   ├── mod.rs                   # Module root
│   ├── schema.rs                # SQLite DDL + migrations
│   ├── store.rs                 # Event store (append, read, replay)
│   ├── state.rs                 # Workflow state machine
│   ├── engine.rs                # Core dispatch loop (adapted from temporal_worker.rs)
│   ├── activities.rs            # Activity executor (tokio tasks + retry)
│   ├── timers.rs                # Persistent timer management
│   ├── signals.rs               # Signal buffering and delivery
│   ├── queries.rs               # Query handler dispatch
│   ├── cancel.rs                # Cancellation propagation
│   └── snapshot.rs              # Checkpointing and compaction
├── lua/
│   └── builtins/
│       ├── workflow.rs          # NEW: workflow.* Lua API
│       ├── temporal.rs          # EXISTING: kept for backward compat
│       └── temporal_worker.rs   # EXISTING: kept for backward compat
└── ...

tests/
├── workflow_basic.rs            # MVP tests: define, start, activities, timers
├── workflow_signals.rs          # Signal + query tests
├── workflow_crash_recovery.rs   # Crash recovery tests (kill + replay)
└── workflow_child.rs            # Child workflow + cancellation tests

docs/modules/
└── workflow.md                  # API documentation (single source of truth)
```

## Testing Strategy

1. **Unit tests**: Each engine component (store, state machine, timer, etc.)
2. **Integration tests**: Full workflow lifecycle via `run_lua()` helper
3. **Crash recovery tests**: Start workflow, simulate crash (drop SQLite connection), resume, verify state
4. **Replay tests**: Start workflow, generate events, replay from scratch, verify identical outcomes
5. **Concurrency tests**: Multiple workflows running simultaneously

## Estimated Effort

| Phase | Lines of Rust | Estimated Time |
|---|---|---|
| Phase 1 (MVP) | ~800-1000 | 3-4 sessions |
| Phase 2 (Signals/Queries) | ~400 | 1-2 sessions |
| Phase 3 (Child/Cancel) | ~300 | 1-2 sessions |
| Phase 4 (Durability/Ops) | ~300 | 1-2 sessions |
| **Total** | **~1800-2000** | **6-10 sessions** |

## Risks and Mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| Coroutine state serialization (for snapshots) | Medium | Use event replay as primary recovery; snapshots are optimization |
| SQLite write contention under high load | Low | Single-process assumption means one writer. WAL mode handles this. |
| Breaking existing temporal users | Low | `temporal` feature flag stays. `workflow` is additive. Clean migration guide. |
| Scope creep (trying to match all Temporal features) | High | Strict MVP. Phase 1 must work end-to-end before Phase 2 starts. |

## Existing Projects for Reference

| Project | Language | Backend | Relevance |
|---|---|---|---|
| [duroxide](https://github.com/microsoft/duroxide) | Rust | SQLite | Microsoft's durable execution framework. SQLite provider. |
| [Persistasaurus](https://github.com/gunnarmorling/persistasaurus) | Java | SQLite | Durable execution with `execution_log` table pattern. |
| [temporalite](https://github.com/temporalio/temporalite) | Go | SQLite | Embedded Temporal server for local dev. SQLite-backed. |
| [DBOS](https://github.com/dbos-inc/dbos-transact) | TypeScript | Postgres | Database-oriented OS. Durable execution via DB transactions. |
| [Restate](https://github.com/restatedev/restate) | Rust | RocksDB | Journal-based durable execution. Similar coroutine model. |
