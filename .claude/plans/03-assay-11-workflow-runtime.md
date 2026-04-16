# Plan: Phase 9 — Workflow Orchestration Runtime (deterministic replay)

## Why this plan exists

The substrate built in Phases 1–8 (data model, REST API, persistence, namespaces, dashboard, auth,
multi-instance Postgres) is real. The runtime that turns that substrate into an executing workflow
engine **was never built**. Phases 1–7 in `02-assay-11-native-workflow-engine.md` were marked ✅ but
only the data-model + API surface exists. There is no orchestration code path that schedules
activities or progresses workflows. Tests verify CRUD; nothing verifies that a workflow actually
runs.

This plan delivers the missing runtime — including deterministic replay so worker crashes don't lose
work — as part of `0.11.1`. Nothing is deferred to a future phase.

## Acceptance contract for the whole phase

The phase is done when **every** test in `crates/assay-workflow/tests/orchestration.rs` passes
against a fresh `assay serve`:

```
1. workflow_runs_to_completion          — 2 sequential activities, end-to-end
2. workflow_with_parallel_activities    — 3 activities started in parallel, results joined
3. workflow_retries_failed_activity     — fails twice, succeeds on attempt 3
4. workflow_with_signal                 — workflow waits for signal; signal arrives; completes
5. workflow_cancellation_stops_work     — cancel propagates; no further activities scheduled
6. workflow_with_durable_timer          — sleep(2) actually pauses 2s and resumes correctly
7. child_workflow_completes_before_parent
8. cron_schedule_fires_workflow         — schedule fires; workflow runs; result persisted
9. worker_crash_resumes_workflow        — kill the worker mid-flight; another worker picks up;
                                          workflow completes with no duplicate side effects
10. side_effect_is_recorded_once        — non-deterministic op cached on first call;
                                          replay returns the cached value
```

If any test fails, the phase is not done. No checkmarks without code.

The same suite gated on `#[cfg(feature = "postgres-test")]` runs against testcontainers Postgres so
we cover both backends.

## The execution model

### Determinism by replay (Temporal-style, simplified)

Workflow code is a Lua function. Each call to a `ctx:` method is assigned a sequence number based on
call order. The engine persists every "completed" command (activity result, timer fire, signal
received, side effect value) as an event with that sequence number.

When a worker is given a workflow task to run:

1. The engine sends `{workflow_id, type, input, history}` (history = all past events).
2. The worker invokes the registered handler in a coroutine.
3. Each `ctx:execute_activity(name, input)` call:
   - Increments a per-execution counter `seq`
   - Searches `history` for an `ActivityCompleted` (or `ActivityFailed`) event at this `seq`
   - If found: returns the cached result (or raises the cached error). **No engine call.**
   - If not found: yields a `ScheduleActivity` command back to the worker.
4. The worker collects yielded commands and POSTs them to the engine.
5. The engine durably writes the corresponding `*Scheduled` events and creates rows
   (`workflow_activities`, `workflow_timers`).
6. When work completes (activity, timer, signal), the engine appends the corresponding
   `*Completed`/`Failed`/`Received` event with the same `seq`.
7. The workflow task is marked dispatchable again; some worker (any worker on the queue) claims it
   and replays from `seq=0`.
8. Replay reaches the previously yielded position, finds the new event in history, returns the
   cached value, and proceeds. Eventually the handler returns → `WorkflowCompleted` → the task is
   removed from the dispatch queue.

### Crash safety

- **Workflow worker dies mid-replay**: the workflow task's `last_heartbeat` ages out
  (`WORKFLOW_TASK_HEARTBEAT_TIMEOUT_SECS = 30`); the engine reassigns it to any other available
  worker, which replays from the event log. No duplicate side effects because all past commands are
  in `history` and short-circuit during replay.
- **Activity worker dies mid-execution**: the activity's `last_heartbeat` ages out
  (`heartbeat_timeout_secs` per-activity); the engine re-queues per the retry policy.
- **Engine dies**: all state is in the DB (SQLite or Postgres). On restart, in-flight workflow tasks
  become claimable again as their heartbeats age out.

### Non-determinism

Non-deterministic operations (current time, random IDs, external HTTP) must go through
`ctx:side_effect("name", function() ... end)` so the result is captured in the event log on first
execution and returned from cache on replay.

## Steps

### 9.1 — Activity scheduling (server side)

| #     | What                                                                                                                                                                                                                                                                                                                                                                                      | Files                                   |
| ----- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------- |
| 9.1.1 | New endpoint `POST /api/v1/workflows/:id/activities` — schedule an activity. Body: `{name, input, task_queue, seq, max_attempts?, initial_interval_secs?, backoff_coefficient?, start_to_close_secs?, heartbeat_timeout_secs?}`. Inserts row in `workflow_activities` with status `PENDING` and appends `ActivityScheduled` event with the given `seq`. Idempotent on (workflow_id, seq). | `api/activities.rs` (new), `api/mod.rs` |
| 9.1.2 | Engine method `schedule_activity(workflow_id, name, input, queue, seq, retry_policy) -> activity_id`. Idempotency: if an activity with this `(workflow_id, seq)` already exists, return its id (no new row, no duplicate event).                                                                                                                                                          | `engine.rs`                             |
| 9.1.3 | Mark workflow `RUNNING` (currently `PENDING` forever) when first activity is scheduled. Already-RUNNING is a no-op.                                                                                                                                                                                                                                                                       | `engine.rs`                             |
| 9.1.4 | New endpoint `GET /api/v1/activities/:id` — return activity record (so workers can poll for completion).                                                                                                                                                                                                                                                                                  | `api/activities.rs`                     |
| 9.1.5 | Test in `orchestration.rs`: POST schedule → row exists → GET returns it → dashboard shows it                                                                                                                                                                                                                                                                                              | `tests/orchestration.rs` (new)          |

### 9.2 — Activity completion → event wiring + retries

| #     | What                                                                                                                                                                                                                            | Files                                               |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------- |
| 9.2.1 | `complete_activity(id, result)` appends `ActivityCompleted` event (with the activity's `seq`) to the workflow event log                                                                                                         | `engine.rs`, `api/tasks.rs`                         |
| 9.2.2 | `fail_activity(id, error)` honors retry policy: if `attempt < max_attempts`, requeues with `scheduled_at = now + backoff`. Otherwise appends `ActivityFailed` event.                                                            | `engine.rs`, `store/sqlite.rs`, `store/postgres.rs` |
| 9.2.3 | Wire heartbeat timeout: an in-flight activity whose `last_heartbeat` is older than `heartbeat_timeout_secs` is auto-failed and retried per policy. The store query already exists — add a periodic poller in `engine::start()`. | `engine.rs`                                         |
| 9.2.4 | When the workflow's pending dispatchable events change (activity completed, timer fired, signal arrived), set `workflow.needs_dispatch = true` so the workflow task becomes claimable                                           | `engine.rs`, schema migration                       |
| 9.2.5 | Test: schedule → claim → complete → ActivityCompleted appears with right seq                                                                                                                                                    | `tests/orchestration.rs`                            |
| 9.2.6 | Test: schedule with `max_attempts=3` → fail twice → claim attempt #3 succeeds → workflow has 1 ActivityCompleted + 2 ActivityRetryQueued events (not 2 ActivityFailed)                                                          | `tests/orchestration.rs`                            |

### 9.3 — Workflow task dispatch (the orchestration loop)

The new "workflow task" model: a `workflow_tasks` table (or a
`needs_dispatch + claimed_by + last_heartbeat` triple on the existing `workflows` table)
representing "this workflow has new events that need a worker to process."

| #     | What                                                                                                                                                                                                                                              | Files                                  |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------- |
| 9.3.1 | Add `needs_dispatch BOOLEAN`, `dispatch_claimed_by TEXT`, `dispatch_last_heartbeat REAL` columns to `workflows` table (migration).                                                                                                                | `store/sqlite.rs`, `store/postgres.rs` |
| 9.3.2 | New endpoint `POST /api/v1/workflow-tasks/poll` — body `{queue, worker_id}`. Returns `{workflow_id, type, input, history}` for an unclaimed dispatchable workflow on that queue. Atomic UPDATE...RETURNING.                                       | `api/workflow_tasks.rs` (new)          |
| 9.3.3 | New endpoint `POST /api/v1/workflow-tasks/:id/heartbeat` — extends the dispatch lease.                                                                                                                                                            | `api/workflow_tasks.rs`                |
| 9.3.4 | New endpoint `POST /api/v1/workflow-tasks/:id/commands` — body `{commands: [...]}`. Engine processes each command transactionally: schedules activities/timers/children, OR marks workflow COMPLETED/FAILED with result. Releases dispatch claim. | `api/workflow_tasks.rs`, `engine.rs`   |
| 9.3.5 | Workflow task heartbeat poller in `engine::start()`: any task whose `dispatch_last_heartbeat` is older than `WORKFLOW_TASK_HEARTBEAT_TIMEOUT_SECS` is released (claim cleared, `needs_dispatch=true`) so another worker picks it up.              | `engine.rs`                            |
| 9.3.6 | When `start_workflow` is called: mark `needs_dispatch=true`, append `WorkflowStarted` event.                                                                                                                                                      | `engine.rs`                            |
| 9.3.7 | When activity completes / timer fires / signal arrives: mark `needs_dispatch=true` on the parent workflow.                                                                                                                                        | `engine.rs`                            |

### 9.4 — Lua deterministic-replay runtime

| #      | What                                                                                                                                                                                                                                                                           | Files                                                       |
| ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------- |
| 9.4.1  | `workflow.define(name, handler)` — actually wired up; handler stored for later execution                                                                                                                                                                                       | `stdlib/workflow.lua`                                       |
| 9.4.2  | Build a `ctx` object inside the workflow runner. Each `ctx:` method increments a `seq` counter and either returns the cached event from `history` or yields a command via `coroutine.yield`.                                                                                   | `stdlib/workflow.lua`                                       |
| 9.4.3  | `ctx:execute_activity(name, input, opts?)` — sequencer + cache lookup + yield                                                                                                                                                                                                  | `stdlib/workflow.lua`                                       |
| 9.4.4  | `ctx:sleep(seconds)` — durable timer via TimerScheduled / TimerCompleted                                                                                                                                                                                                       | `stdlib/workflow.lua`                                       |
| 9.4.5  | `ctx:wait_for_signal(name)` — looks for `WorkflowSignaled` event with matching name at any seq ≥ current; if not present, yields `WaitForSignal`                                                                                                                               | `stdlib/workflow.lua`                                       |
| 9.4.6  | `ctx:side_effect(name, function() ... end)` — for non-deterministic operations. First call: runs the function, yields `RecordSideEffect` with the value; engine writes `SideEffectRecorded` event. Replay: returns the cached value from history without running the function. | `stdlib/workflow.lua`, `engine.rs`, `api/workflow_tasks.rs` |
| 9.4.7  | `ctx:start_child_workflow(type, opts)` — yields ChildWorkflowScheduled command; replay returns child workflow handle from event                                                                                                                                                | `stdlib/workflow.lua`                                       |
| 9.4.8  | `ctx:now()` and `ctx:rand()` — convenience helpers built on `side_effect` so they're replay-safe                                                                                                                                                                               | `stdlib/workflow.lua`                                       |
| 9.4.9  | Workflow runner: pulls workflow tasks via `POST /workflow-tasks/poll`, runs the handler in a coroutine, batches yielded commands, posts to `/workflow-tasks/:id/commands`.                                                                                                     | `stdlib/workflow.lua`                                       |
| 9.4.10 | `workflow.listen({queue})` polls BOTH workflow tasks AND activity tasks in the same loop (workflow tasks first because they're cheap orchestration)                                                                                                                            | `stdlib/workflow.lua`                                       |
| 9.4.11 | If a workflow handler raises an error, the runner sends a `FailWorkflow` command with the error. Engine appends `WorkflowFailed` event and marks workflow `FAILED`.                                                                                                            | `stdlib/workflow.lua`, `engine.rs`                          |

### 9.5 — Timers

| #     | What                                                                                                                                                                                                            | Files                                  |
| ----- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------- |
| 9.5.1 | New table `workflow_timers (id, workflow_id, seq, fires_at, status)` migration                                                                                                                                  | `store/sqlite.rs`, `store/postgres.rs` |
| 9.5.2 | When a worker yields `ScheduleTimer{seq, seconds}`, engine creates row with `fires_at = now + seconds` and appends `TimerScheduled` event                                                                       | `engine.rs`                            |
| 9.5.3 | Timer poller in `engine::start()`: every second, finds rows where `fires_at <= now AND status='PENDING'`, marks them `FIRED`, appends `TimerCompleted` event, sets `needs_dispatch=true` on the parent workflow | `engine.rs`                            |
| 9.5.4 | Cancellation: when a workflow is cancelled, its pending timers are marked `CANCELLED`                                                                                                                           | `engine.rs`                            |

### 9.6 — Signal handling integrated with replay

| #     | What                                                                                                                                             | Files                    |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------ |
| 9.6.1 | `send_signal` already appends `WorkflowSignaled` event; ensure it also sets `needs_dispatch=true` so a waiting workflow wakes up                 | `engine.rs`              |
| 9.6.2 | Replay logic: `ctx:wait_for_signal(name)` scans history for matching signal event after current seq; if found, returns payload; otherwise yields | `stdlib/workflow.lua`    |
| 9.6.3 | Test: workflow waits → signal sent → workflow resumes with payload                                                                               | `tests/orchestration.rs` |

### 9.7 — Cancellation propagation through replay

| #     | What                                                                                                                                                                                                                       | Files                    |
| ----- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------ |
| 9.7.1 | When `cancel_workflow` is called: append `WorkflowCancelRequested` event, mark `needs_dispatch=true`                                                                                                                       | `engine.rs`              |
| 9.7.2 | Replay: every `ctx:` method checks for `WorkflowCancelRequested` in history; if present and no `WorkflowCancelHandled` later, raises a Lua error that bubbles up. The runner sees this and sends `CancelWorkflow` command. | `stdlib/workflow.lua`    |
| 9.7.3 | Engine cancels pending activities, timers, child workflows of a cancelled workflow                                                                                                                                         | `engine.rs`              |
| 9.7.4 | Test: cancel running workflow with pending activity → activity goes CANCELLED → workflow ends CANCELLED                                                                                                                    | `tests/orchestration.rs` |

### 9.8 — Cron scheduler fires real workflows

| #     | What                                                                                                                                                             | Files                    |
| ----- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------ |
| 9.8.1 | When scheduler fires a schedule, it calls `start_workflow` which now properly queues a workflow task. No code change needed here once 9.3 is done — only verify. | `scheduler.rs`           |
| 9.8.2 | Test: create schedule with `* * * * *` → wait ≤65s → workflow exists with status `COMPLETED` and result                                                          | `tests/orchestration.rs` |

### 9.9 — Real integration tests (the gate)

`crates/assay-workflow/tests/orchestration.rs` — each test boots a real engine **and** a real Lua
worker (in another tokio task or as a child process) and runs a real workflow.

The 10 tests listed in the acceptance contract above. Each test must:

- Start engine in-process
- Spawn a worker that calls `workflow.listen({queue})`
- Register at least one workflow handler
- Trigger the scenario via REST API
- Poll until terminal state (with timeout)
- Assert workflow.status, workflow.result, and the event log are correct

The test harness lives in `tests/common/mod.rs` (new) and provides:

- `start_engine() -> EngineHandle`
- `spawn_lua_worker(engine_url, queue, registrations) -> WorkerHandle` (forks
  `assay run worker.lua`)
- `wait_for_workflow_status(engine_url, id, status, timeout) -> WorkflowRecord`

The Postgres variant uses testcontainers and is gated by feature.

### 9.10 — Dashboard reality check

| #      | What                                                                                                                     |
| ------ | ------------------------------------------------------------------------------------------------------------------------ |
| 9.10.1 | Workflow detail panel: confirm activity timeline shows real `ActivityScheduled/Completed/Failed` events with real timing |
| 9.10.2 | Worker view: confirm "active tasks" shows real claimed activities AND claimed workflow tasks                             |
| 9.10.3 | Queue view: confirm pending/running counts are real for both queues                                                      |
| 9.10.4 | Add a "side effects" section to the workflow detail (shows recorded values)                                              |
| 9.10.5 | Add a "timers" section showing pending and fired timers                                                                  |

### 9.11 — Honest plan + docs + examples

| #      | What                                                                                                                                                   | Files                                   |
| ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------- |
| 9.11.1 | Update `02-assay-11-native-workflow-engine.md`: change Phases 1–7 from "✅ complete" to "✅ scaffolded; runtime in Phase 9". Insert Phase 9 reference. | plan doc                                |
| 9.11.2 | Update `docs/modules/workflow.md` to match what Phase 9 ships, including the deterministic-replay model and crash-safety guarantees                    | `docs/modules/workflow.md`              |
| 9.11.3 | `examples/workflows/` — written ONLY AFTER 9.9 passes. Examples are real, runnable, and verified by CI.                                                | `examples/workflows/`, `site/build.lua` |
| 9.11.4 | Update CHANGELOG `[0.11.1]` to honest scope: "workflow engine with deterministic-replay runtime"                                                       | `CHANGELOG.md`                          |

### 9.12 — Site update for examples + release banner

| #      | What                                                                                |
| ------ | ----------------------------------------------------------------------------------- |
| 9.12.1 | Extend `site/build.lua` to render `examples/workflows/*/README.md` into the website |
| 9.12.2 | Add `0.11.1 released` callout on the homepage                                       |
| 9.12.3 | Modernize the homepage hero (separate ticket, after 0.11.1 ships)                   |

## Implementation order

Strict order — each step has a passing test before moving to the next:

1. **9.1.1 + 9.1.2 + 9.1.5** — schedule activity endpoint + first test passing
2. **9.2.1 + 9.2.5** — complete activity → event firing test
3. **9.2.2 + 9.2.6** — retry test
4. **9.3.1 + 9.3.2 + 9.3.6 + 9.3.7** — workflow task dispatch (no Lua yet, test from REST level)
5. **9.4.1–9.4.3 + 9.4.9 + 9.4.10** — Lua coroutine runtime, plus `workflow_runs_to_completion` test
6. **9.5** — timers, plus `workflow_with_durable_timer` test
7. **9.6** — signals, plus `workflow_with_signal` test
8. **9.7** — cancellation, plus `workflow_cancellation_stops_work` test
9. **9.4.7** — child workflows, plus `child_workflow_completes_before_parent` test
10. **9.4.6** — side effects, plus `side_effect_is_recorded_once` test
11. **9.3.3 + 9.3.5** — workflow task heartbeat / takeover, plus `worker_crash_resumes_workflow`
    test
12. **9.8** — cron firing real workflows, plus `cron_schedule_fires_workflow` test
13. **9.10** — dashboard verification (manual + new component-level smoke tests)
14. **9.11 + 9.12.1** — docs, examples, site build extension
15. **Tag `v0.11.1`**, publish crates, push to GHCR

Each step lands as one or more commits on `feat/workflow-engine`. The branch does not merge to
`main` until step 15.

## Sizing (rough)

| Area                                                                        | Lines           |
| --------------------------------------------------------------------------- | --------------- |
| `api/activities.rs` + `api/workflow_tasks.rs`                               | ~400            |
| `engine.rs` runtime additions (dispatch, timers, retry, cancel propagation) | ~500            |
| `stdlib/workflow.lua` (deterministic-replay coroutine runner)               | ~400            |
| `store` migrations + new queries (timers, dispatch fields)                  | ~200            |
| `tests/orchestration.rs` + `tests/common/mod.rs`                            | ~700            |
| Docs + plan + CHANGELOG                                                     | ~300            |
| **Total**                                                                   | **~2500 lines** |

## What we are explicitly NOT doing

Everything I'd be tempted to defer is in scope. The only things genuinely out of scope:

- **Workflow versioning** — when a deployed workflow's code changes mid-run, we don't yet handle
  "version 1 history must continue with version 1 logic." Plan: a `workflow_version` column added
  later. Not blocking 0.11.1.
- **Distributed tracing / OpenTelemetry export** — workflow events are visible via dashboard
  - REST. OTLP integration is later.
- **Long-poll on workflow-tasks/poll** — the first cut uses 1-second short-poll. Later we add a
  `wait` query param that holds the connection until a task is available.

These are listed so we know explicitly what we're skipping; everything else in this plan ships in
0.11.1.
