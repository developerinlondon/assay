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

The `temporalio-sdk` crate (already a dependency) includes the `Worker` type. No new crate
dependencies are needed.

### Architecture

```
temporal.worker(opts)
        |
        v
  Rust: create Worker (temporalio-sdk)
        |
        +-- register activity functions
        |     \-- each wraps a Lua function: deserialize JSON input,
        |         call Lua fn, serialize JSON output
        |
        +-- register workflow functions
        |     \-- each wraps a Lua function with a deterministic ctx:
        |         ctx:execute_activity -> schedules activity in Temporal
        |         ctx:wait_signal -> blocks on Temporal signal channel
        |         ctx:sleep -> Temporal timer
        |
        +-- worker.run() in background tokio task
              \-- polls task queue, dispatches to registered fns
```

### Activities (straightforward)

Activities are non-deterministic — they're just functions. The Rust bridge:

1. Receives an activity task from Temporal (JSON payload)
2. Calls the registered Lua function with the deserialized input
3. Serializes the Lua return value as JSON
4. Returns the result to Temporal

### Workflows (deterministic replay)

Workflows must be deterministic for Temporal's replay mechanism. The Rust bridge:

1. Provides a `ctx` userdata object to the Lua workflow function
2. `ctx:execute_activity()` records a command in the workflow history and blocks until the activity
   completes
3. `ctx:wait_signal()` records a signal wait and blocks until the signal arrives
4. On replay, the bridge replays from history instead of re-executing

This follows the same pattern as the Go and Python SDKs — the workflow function runs "as if" it's
executing fresh, but the `ctx` methods short-circuit using history on replay.

## Binary size impact (measured)

| Build                                     | Size     | Delta              |
| ----------------------------------------- | -------- | ------------------ |
| Without temporal feature                  | 6.8MB    | baseline           |
| With temporal client (current shipping)   | 8.7MB    | +1.9MB             |
| With temporal client + worker (estimated) | ~10-11MB | +1-2MB over client |

The `temporalio-sdk` Worker type shares the same protobuf/gRPC stack already linked by the client.
The incremental cost for worker support is the polling loop, activity/workflow dispatch, and replay
engine — estimated at 1-2MB additional over the current 8.7MB.

Total estimated binary with full worker support: **~10-11MB**.

## Alternatives considered

1. **External Go/TypeScript worker** — works today but adds operational complexity and a second
   language. Defeats assay's single-binary value proposition.

2. **Activity-only worker (no workflow support)** — simpler to implement but loses Temporal's core
   value: durable orchestration with replay. Users would still need external workflow definitions.

3. **Signal-driven workflows only** — pre-built Rust workflow templates driven by signals. Simpler
   replay story but less flexible — can't define arbitrary workflow logic in Lua.
