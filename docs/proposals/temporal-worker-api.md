# Proposal: Temporal Worker API for Assay

## Summary

Add `temporal.worker()` to assay's native Temporal integration, enabling Lua scripts to register as
Temporal workers that execute workflows and activities. Currently assay provides a Temporal
**client** (start, signal, query, cancel workflows) but no **worker** runtime (poll task queues,
execute workflow/activity code).

## Motivation

Without a worker, `start_workflow` puts a task on the queue but nothing processes it. Users must
deploy a separate Go/TypeScript/Python service as the worker, which:

- Adds operational complexity (separate build, deploy, monitor)
- Duplicates logic between the Lua app and the worker
- Defeats the single-binary advantage of assay

A native Lua worker keeps everything in one binary and one language.

---

## Full Temporal API Surface in Assay

After this work, assay covers three complementary APIs:

```
+-------------------------------------------------------------------+
|                        assay.temporal                              |
|                                                                   |
|  1. HTTP REST client        require("assay.temporal")             |
|     Read-only monitoring:   list, query, search, history,         |
|                             schedules, task queues                |
|                                                                   |
|  2. Native gRPC client      temporal.connect(opts)                |
|     Start + interact:       start_workflow, signal_workflow,      |
|                             query_workflow, cancel, terminate,    |
|                             describe, get_result                  |
|                                                                   |
|  3. Native gRPC worker      temporal.worker(opts)                 |
|     Execute:                activities (plain Lua functions),     |
|                             workflows (Lua functions with         |
|                             deterministic ctx)                    |
+-------------------------------------------------------------------+
```

A typical deployment uses (2) to start workflows and (3) to execute them. (1) is for dashboards
and monitoring tools that don't need gRPC.

---

## Proposed API

### Starting a worker

```lua
local handle = temporal.worker({
  url = "temporal-frontend:7233",
  namespace = "command-center",
  task_queue = "promotions",

  -- Activities: plain Lua functions that do real I/O.
  -- Each receives a single input value (deserialized from JSON) and returns
  -- a result (serialized to JSON). Activities are retried by Temporal on failure.
  activities = {
    update_gitops = function(input)
      -- Call GitLab API to update overlay files
      local resp = http.request("POST", input.gitlab_url, {
        body = json.encode(input.commit),
        headers = { ["PRIVATE-TOKEN"] = input.token },
      })
      if resp.status ~= 201 then
        error("GitLab commit failed: HTTP " .. resp.status)
      end
      return json.parse(resp.body)
    end,

    poll_argocd = function(input)
      -- Poll ArgoCD until apps are synced + healthy
      local resp = http.get(input.argocd_url .. "/api/v1/applications/" .. input.app)
      local app = json.parse(resp.body)
      return {
        synced = app.status.sync.status == "Synced",
        healthy = app.status.health.status == "Healthy",
      }
    end,

    notify = function(input)
      http.post(input.webhook, { text = input.message })
      return { sent = true }
    end,
  },

  -- Workflows: orchestration functions that coordinate activities.
  -- The ctx object provides deterministic primitives. Temporal replays
  -- these functions from history — the ctx methods handle the replay
  -- transparently so the Lua code reads like normal sequential code.
  workflows = {
    PromotionWorkflow = function(ctx, input)
      local info = ctx:workflow_info()
      log.info("Promotion " .. info.workflow_id .. " started: "
        .. input.version .. " -> " .. input.target)

      -- Stage 1: Wait for human approval (blocks until signal or timeout)
      local approval = ctx:wait_signal("approve", { timeout = 86400 })
      if not approval then
        ctx:execute_activity("notify", {
          webhook = input.webhook,
          message = "Promotion " .. input.version .. " timed out waiting for approval",
        })
        return { status = "timed_out" }
      end

      -- Stage 2: Update GitOps overlays
      local commit = ctx:execute_activity("update_gitops", {
        gitlab_url = input.gitlab_url,
        token = input.gitlab_token,
        commit = input.gitops_commit,
      }, {
        start_to_close_timeout = 30,
        retry_policy = { maximum_attempts = 3 },
      })

      -- Stage 3: Wait for ArgoCD sync (poll with deterministic sleep)
      local synced = false
      for i = 1, 30 do
        local status = ctx:execute_activity("poll_argocd", {
          argocd_url = input.argocd_url,
          app = input.app_name,
        }, { start_to_close_timeout = 10 })

        if status.synced and status.healthy then
          synced = true
          break
        end
        ctx:sleep(10)  -- deterministic: uses Temporal timer, not wall clock
      end

      if not synced then
        ctx:execute_activity("notify", {
          webhook = input.webhook,
          message = "Promotion " .. input.version .. " failed: ArgoCD sync timeout",
        })
        return { status = "failed", reason = "argocd_sync_timeout" }
      end

      -- Stage 4: Notify success
      ctx:execute_activity("notify", {
        webhook = input.webhook,
        message = "Promotion " .. input.version .. " deployed to " .. input.target,
      })

      return {
        status = "done",
        version = input.version,
        target = input.target,
        commit_id = commit.short_id,
        approved_by = approval.user,
      }
    end,
  },
})

-- The handle lets you inspect and shut down the worker
print(handle:is_running())  -- true
-- handle:shutdown()         -- graceful: drains in-flight tasks
```

### Starting a workflow (from another script or the same app)

```lua
temporal.connect({
  url = "temporal-frontend:7233",
  namespace = "command-center",
})

-- Start a promotion workflow
local run = temporal.start_workflow("PromotionWorkflow", {
  task_queue = "promotions",
  workflow_id = "promote-v0.2.0-to-prod",
  input = {
    version = "v0.2.0",
    target = "prod",
    gitlab_url = "https://gitlab.example.com/api/v4/projects/123/repository/commits",
    gitlab_token = env.get("GITLAB_TOKEN"),
    gitops_commit = { ... },
    argocd_url = "https://argocd.example.com",
    app_name = "simons-core-api-prod",
    webhook = "https://hooks.slack.com/...",
  },
})

-- Later: approve via signal
temporal.signal_workflow("promote-v0.2.0-to-prod", "approve", {
  user = "jane.smith@example.com",
})

-- Wait for result
local result = temporal.get_result("promote-v0.2.0-to-prod")
```

### Monitoring (from a dashboard)

```lua
local t = require("assay.temporal").client("http://temporal-frontend:8080", {
  namespace = "command-center",
})

-- List running promotions
local running = t:workflows({ query = 'WorkflowType = "PromotionWorkflow"' })

-- Check specific workflow
local wf = t:workflow("promote-v0.2.0-to-prod")
print(wf.workflowExecutionInfo.status)

-- Get history for debugging
local history = t:workflow_history("promote-v0.2.0-to-prod")
```

---

## Workflow Context (`ctx`) Reference

The `ctx` object is a Lua userdata backed by Temporal's workflow activation mechanism.
Every method is deterministic — on replay, results come from history instead of re-executing.

### ctx:execute_activity(name, input, opts?)

Schedule a registered activity and block until it completes.

```lua
local result = ctx:execute_activity("update_gitops", {
  target = "prod",
  version = "v0.2.0",
}, {
  start_to_close_timeout = 300,   -- seconds (required)
  schedule_to_close_timeout = 600, -- overall deadline including queue time
  heartbeat_timeout = 30,          -- activity must heartbeat within this interval
  retry_policy = {
    initial_interval = 1,          -- seconds between retries
    backoff_coefficient = 2.0,     -- exponential backoff multiplier
    maximum_interval = 60,         -- cap on retry interval
    maximum_attempts = 5,          -- 0 = unlimited
    non_retryable_errors = { "PERMISSION_DENIED" },
  },
})
```

Returns the activity's return value (deserialized from JSON). Throws on activity failure
after retries are exhausted.

### ctx:wait_signal(name, opts?)

Block until an external signal is received or timeout expires.

```lua
-- Block indefinitely
local payload = ctx:wait_signal("approve")

-- Block with timeout (returns nil on timeout)
local payload = ctx:wait_signal("approve", { timeout = 86400 })

-- The payload is whatever the signaller sent:
--   temporal.signal_workflow("wf-id", "approve", { user = "jane" })
-- So payload = { user = "jane" }
```

Signals are buffered by Temporal — if a signal arrives before `wait_signal` is called,
it's delivered immediately on the next activation.

### ctx:sleep(seconds)

Deterministic sleep using a Temporal timer. On replay, returns immediately if the timer
already fired in history.

```lua
ctx:sleep(60)  -- wait 1 minute (Temporal timer, not wall clock)
```

### ctx:side_effect(fn)

Run a non-deterministic function exactly once. On replay, the recorded result is returned
instead of re-executing the function. Use for things like generating UUIDs or reading
wall-clock time inside a workflow.

```lua
local id = ctx:side_effect(function()
  return crypto.random(16)
end)

local now = ctx:side_effect(function()
  return os.date("!%Y-%m-%dT%H:%M:%SZ")
end)
```

### ctx:workflow_info()

Returns metadata about the current workflow execution.

```lua
local info = ctx:workflow_info()
-- {
--   workflow_id = "promote-v0.2.0-to-prod",
--   run_id = "abc123-def456",
--   namespace = "command-center",
--   task_queue = "promotions",
--   attempt = 1,
--   workflow_type = "PromotionWorkflow",
-- }
```

---

## Implementation (Rust side)

### Feature flag

```toml
[features]
temporal = ["dep:temporalio-client", "dep:temporalio-sdk", "dep:temporalio-common"]
# ^^^ already exists, worker support added here (shared crates)
```

Uses `temporalio-sdk-core` (the low-level Core SDK) rather than the high-level `temporalio-sdk`
Worker type. The high-level SDK uses proc macros for activity/workflow registration which don't
work with dynamic Lua function dispatch. The Core SDK gives direct control over:

- Task polling (`poll_activity_task`, `poll_workflow_activation`)
- Activity completion (`complete_activity_task`)
- Workflow activation completion (`complete_workflow_activation`)
- Worker lifecycle (`init_worker`, `initiate_shutdown`)

This is the same approach the Python and .NET SDKs use — they bridge the Core SDK to their
respective runtimes rather than using the Rust-native high-level abstractions.

### Architecture

```
temporal.worker(opts)
        |
        v
  Rust: init_worker() via temporalio-sdk-core
        |
        +-- Activity polling loop (tokio task)
        |     poll_activity_task()
        |       -> channel -> Lua dispatcher (spawn_local)
        |         -> lookup registered Lua fn by activity_type
        |         -> deserialize JSON payload -> call Lua fn -> serialize result
        |       -> channel -> completer (tokio task)
        |         -> complete_activity_task()
        |
        +-- Workflow polling loop (tokio task)
        |     poll_workflow_activation()
        |       -> channel -> Lua dispatcher (spawn_local)
        |         -> per-workflow coroutine (created on StartWorkflow job)
        |         -> resume coroutine with activation jobs
        |         -> ctx methods yield commands back to Rust
        |       -> channel -> completer (tokio task)
        |         -> complete_workflow_activation()
        |
        +-- Handle returned to Lua
              handle:shutdown()   -- initiate graceful shutdown
              handle:is_running() -- check if worker is still active
```

### Activities (implemented)

Activities are non-deterministic — they're just functions. The Rust bridge:

1. Polls `poll_activity_task()` on a tokio task
2. Sends `(activity_type, input_payloads, task_token)` via unbounded channel to the Lua dispatcher
3. Lua dispatcher runs on `spawn_local` (same async context as the Lua VM):
   - Looks up the registered Lua function by `activity_type`
   - Deserializes the first input payload as JSON → Lua value
   - Calls the Lua function
   - Serializes the return value as JSON → Temporal Payload
4. Sends `(task_token, ActivityExecutionResult)` via channel to the completer
5. Completer calls `complete_activity_task()` to report success/failure to Temporal

The channel-based architecture keeps the poller and completer on regular tokio tasks (Send)
while the Lua dispatch stays on `spawn_local` (Lua is !Send).

### Workflows (to implement)

The workflow bridge is more complex than activities because of Temporal's deterministic replay
model. Here's the detailed design:

#### Core concept: activations, not direct execution

Temporal doesn't call a workflow function once. It sends **activations** — batches of **jobs**
that happened since the last activation. Each activation is a list of events:

```
Activation 1: [StartWorkflow { input }]
Activation 2: [ResolveActivity { result }]
Activation 3: [SignalWorkflow { signal_name, payload }]
Activation 4: [ResolveActivity { result }, FireTimer {}]
```

The worker must:
1. Replay the workflow function from the beginning using cached results for completed commands
2. Execute any new logic that the replay reaches
3. Return a list of **commands** (ScheduleActivity, StartTimer, etc.) for Temporal to execute

#### Per-workflow coroutine model

Each workflow execution gets a Lua coroutine. The coroutine runs the user's workflow function
and **yields** whenever it hits a `ctx` method that needs to wait for an external result:

```
Lua coroutine                          Rust dispatcher
     |                                      |
     |  ctx:execute_activity("foo", input)  |
     |  -------- yield(ScheduleActivity) -->|
     |                                      | sends command to Temporal
     |                                      | ... time passes ...
     |                                      | activation arrives: ResolveActivity
     |  <-- resume(activity_result) --------|
     |                                      |
     |  ctx:wait_signal("approve")          |
     |  -------- yield(WaitSignal) -------->|
     |                                      | ... time passes ...
     |                                      | activation arrives: SignalWorkflow
     |  <-- resume(signal_payload) ---------|
     |                                      |
     |  return { status = "done" }          |
     |  -------- coroutine finishes ------->|
     |                                      | sends CompleteWorkflow command
```

#### Replay handling

On replay (e.g. after worker restart), the Core SDK sends an activation containing ALL
historical events. The Rust dispatcher:

1. Creates a fresh coroutine for the workflow function
2. Maintains a **command index** (which `ctx` call we're on)
3. For each `ctx` yield:
   - If a matching result exists in the activation's resolved events → resume immediately
     with the cached result (replay)
   - If no result exists → this is new work, send the command to Temporal and suspend

This is exactly how the Python SDK's `_WorkflowInstanceImpl` works — it maintains a
`_next_seq` counter and matches commands to results by sequence number.

#### Workflow state management

```rust
struct WorkflowInstance {
    run_id: String,
    coroutine: RegistryKey,        // Lua coroutine stored in registry
    pending_commands: Vec<Command>, // commands to send after activation
    resolved_results: VecDeque<ResolvedResult>, // results from activation
    signals: HashMap<String, VecDeque<Payload>>, // buffered signals by name
}
```

The dispatcher maintains a `HashMap<String, WorkflowInstance>` keyed by run_id.

#### ctx method implementations

Each `ctx` method follows the same pattern:

1. Check if there's a resolved result available (replay) → return immediately
2. Otherwise, push a command and yield the coroutine

```
ctx:execute_activity(name, input, opts)
  → Command::ScheduleActivity { activity_type, input, timeouts, retry }
  → yields, resumed with activity result or failure

ctx:wait_signal(name, opts)
  → check signals buffer first (signal may have arrived already)
  → if buffered: return immediately
  → if not: yield, resumed when SignalWorkflow job arrives
  → if timeout: also push Command::StartTimer, return nil on timer fire

ctx:sleep(seconds)
  → Command::StartTimer { duration }
  → yields, resumed when FireTimer job arrives

ctx:side_effect(fn)
  → on first execution: call fn(), record result, push Command::RecordMarker
  → on replay: read result from marker in history
```

## Binary size impact (measured)

| Build                                     | Size     | Delta              |
| ----------------------------------------- | -------- | ------------------ |
| Without temporal feature                  | 6.8MB    | baseline           |
| With temporal client (current shipping)   | 8.7MB    | +1.9MB             |
| With temporal client + worker (estimated) | ~10-11MB | +1-2MB over client |

The Core SDK shares the same protobuf/gRPC stack already linked by the client. The incremental
cost for worker support is the polling loop, activity/workflow dispatch, and replay engine —
estimated at 1-2MB additional over the current 8.7MB.

Total estimated binary with full worker support: **~10-11MB**.

## Current status

- **Activities**: Fully implemented. Lua functions are registered, polled, dispatched, and
  completed via the Core SDK. Error handling returns `Failure` objects to Temporal.
- **Workflows**: Fully implemented. Lua coroutine bridge dispatches workflow activations,
  with deterministic ctx methods for execute_activity, wait_signal, sleep, side_effect,
  and workflow_info. Replay is handled via sequence-numbered resolved results.
- **Shutdown**: Graceful via `handle:shutdown()`. Worker drains in-flight tasks.

## Implementation plan

```
Phase 1: Workflow polling + coroutine lifecycle         [done]
  - poll_workflow_activation() loop (same channel pattern as activity loop)
  - Create Lua coroutine on InitializeWorkflow job
  - Store WfInstance per run_id with thread + ctx registry keys
  - Complete workflow on coroutine return (CompleteWorkflowExecution)
  - Fail workflow on coroutine error (FailWorkflowExecution)
  - Eviction handling (RemoveFromCache → cleanup instance)

Phase 2: ctx:execute_activity                           [done]
  - ctx as Lua table with methods that yield command tables
  - yield ScheduleActivity command from Lua coroutine
  - Resume coroutine on ResolveActivity job with decoded result
  - Replay: sequence-numbered _resolved table, return cached result without yield
  - Activity failure → _activity_error table → Lua error() in ctx method
  - Retry policy, timeouts parsed from opts table

Phase 3: ctx:wait_signal + ctx:sleep                    [done]
  - Signal buffering (_signals table, signals can arrive before wait_signal)
  - StartTimer command for sleep and signal timeouts
  - Resume on SignalWorkflow (payload) or FireTimer (nil for timeout)
  - CancelTimer issued when signal arrives before timeout

Phase 4: ctx:side_effect + ctx:workflow_info            [done]
  - side_effect: calls fn() directly, best-effort (not persisted via markers)
  - workflow_info: returns table from InitializeWorkflow metadata
  - Sequence numbers always increment for deterministic replay

Phase 5: Error handling + edge cases                    [done]
  - Activity failure propagation (Completed/Failed/Cancelled variants)
  - Workflow cancellation (CancelWorkflow → CancelWorkflowExecution command)
  - SDK-level errors caught and reported as workflow task failures
  - Unhandled activation jobs logged and skipped gracefully
```

### Design note: ctx as Lua table vs Rust userdata

The ctx object is implemented as a plain Lua table with methods that call
`coroutine.yield()`. This was chosen over Rust userdata because:

1. Yield from within a Rust-backed method has complex lifetime interactions
2. Lua tables are transparent — easy to populate `_resolved`/`_signals` from Rust
3. The ctx factory is a single embedded Lua source string (CTX_LUA constant)
4. No `#[userdata]` proc macros needed for the yield/resume pattern

## Alternatives considered

1. **High-level `temporalio-sdk` Worker type** — Originally planned but rejected. The high-level
   SDK uses Rust proc macros (`#[activity]`, `#[workflow]`) for static registration, which
   doesn't work with dynamic Lua function dispatch. The Core SDK (`temporalio-sdk-core`) gives
   the control needed to bridge to Lua's runtime model.

2. **External Go/TypeScript worker** — works today but adds operational complexity and a second
   language. Defeats assay's single-binary value proposition.

3. **Activity-only worker (no workflow support)** — This is effectively the current state.
   Useful for many real workloads, but the full value comes when workflow orchestration lands.

4. **Signal-driven workflows only** — pre-built Rust workflow templates driven by signals.
   Simpler replay story but less flexible — can't define arbitrary workflow logic in Lua.
