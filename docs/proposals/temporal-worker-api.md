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

## Proposed API

### Starting a worker

```lua
temporal.worker({
  url = "temporal-frontend:7233",
  namespace = "my-namespace",
  task_queue = "my-queue",

  -- Activities: plain Lua functions that do actual I/O
  activities = {
    send_email = function(input)
      http.post(input.webhook, { to = input.to, body = input.body })
      return { sent = true }
    end,
    check_status = function(input)
      local resp = http.get(input.url)
      return { status = resp.status }
    end,
  },

  -- Workflows: orchestration functions that call activities and wait for signals.
  -- The ctx object provides deterministic primitives.
  workflows = {
    ApprovalWorkflow = function(ctx, input)
      -- Wait for a human signal (blocks until signalled or timeout)
      local approval = ctx:wait_signal("approve", { timeout = 86400 })
      if not approval then
        return { status = "timed_out" }
      end

      -- Execute activities (retried automatically by Temporal on failure)
      local result = ctx:execute_activity("send_email", {
        to = input.requester,
        body = "Approved!",
      }, { start_to_close_timeout = 60 })

      return { status = "done", result = result }
    end,
  },
})
```

### Workflow context (`ctx`)

The `ctx` object passed to workflow functions provides deterministic primitives:

| Method                                    | Description                                                                     |
| ----------------------------------------- | ------------------------------------------------------------------------------- |
| `ctx:execute_activity(name, input, opts)` | Run a registered activity. Blocks until complete.                               |
| `ctx:wait_signal(name, opts)`             | Block until a signal is received or timeout. Returns signal payload or nil.     |
| `ctx:sleep(seconds)`                      | Deterministic sleep (uses Temporal timer, not wall clock).                      |
| `ctx:side_effect(fn)`                     | Run a non-deterministic function once, replay the result on subsequent replays. |
| `ctx:workflow_info()`                     | Returns `{ workflow_id, run_id, namespace, task_queue, attempt }`.              |

### Activity options

```lua
ctx:execute_activity("name", input, {
  start_to_close_timeout = 300,   -- max seconds for activity to complete
  retry_policy = {
    initial_interval = 1,          -- seconds
    backoff_coefficient = 2.0,
    maximum_interval = 60,
    maximum_attempts = 3,
  },
})
```

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
        +-- Workflow polling loop (TODO — next iteration)
        |     poll_workflow_activation()
        |       -> dispatch activation jobs to Lua coroutine
        |       -> ctx userdata bridges deterministic primitives
        |       -> complete_workflow_activation()
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

### Workflows (next iteration)

Workflows require a deterministic replay bridge. The Rust side will:

1. Poll `poll_workflow_activation()` for activation jobs
2. Provide a `ctx` userdata to the Lua workflow function with deterministic primitives:
   - `ctx:execute_activity()` → schedules a command, blocks until result
   - `ctx:wait_signal()` → blocks until signal received or timeout
   - `ctx:sleep()` → Temporal timer (not wall clock)
   - `ctx:side_effect()` → run once, replay from history
3. On replay, `ctx` methods short-circuit using history instead of re-executing
4. Call `complete_workflow_activation()` with the resulting commands

This follows the same pattern as the Go and Python SDKs — the workflow function runs "as if"
executing fresh, but the `ctx` methods are backed by the Temporal replay mechanism.

The Lua coroutine model maps naturally to this: each workflow activation suspends the coroutine
at `ctx:execute_activity()` / `ctx:wait_signal()`, and resumes when the next activation arrives
with the result.

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
- **Workflows**: Registered but not yet dispatched. The `ctx` coroutine bridge is the next
  piece of work. Activities can be used standalone (many Temporal use cases are activity-only).
- **Shutdown**: Graceful via `handle:shutdown()`. Worker drains in-flight tasks.

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
