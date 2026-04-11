## Temporal

Assay provides three complementary Temporal APIs:

| API | Access | Purpose |
|-----|--------|---------|
| HTTP REST client | `require("assay.temporal")` | Monitoring: list workflows, check status, get history |
| Native gRPC client | `temporal.connect()` | Interact: start, signal, query, cancel workflows |
| Worker runtime | `temporal.worker()` | Execute: poll task queues, run activities and workflows |

All three are available when built with `--features temporal` (enabled by default).

---

## assay.temporal (HTTP REST client)

Read-only monitoring client for Temporal's HTTP API. Use for dashboards and status checks.

Client: `require("assay.temporal").client(url, {namespace?, api_key?})`.

- `c:health()` → bool — Check Temporal health via `/health`
- `c:system_info()` → info — Get Temporal system information
- `c:namespaces()` → `{namespaces}` — List all namespaces
- `c:namespace(name)` → namespace — Get namespace by name
- `c:workflows(opts?)` → `{executions}` — List workflow executions. `opts`: `{namespace, query, page_size}`
- `c:workflow(workflow_id, run_id?, opts?)` → workflow — Get workflow execution details
- `c:workflow_history(workflow_id, run_id?, opts?)` → `{events}` — Get workflow event history. `opts`: `{namespace, maximum_page_size}`
- `c:signal_workflow(workflow_id, signal_name, input?, opts?)` → result — Signal a running workflow. `opts`: `{namespace, run_id}`
- `c:terminate_workflow(workflow_id, reason?, opts?)` → result — Terminate a workflow. `opts`: `{namespace, run_id}`
- `c:cancel_workflow(workflow_id, opts?)` → result — Request workflow cancellation. `opts`: `{namespace, run_id}`
- `c:task_queue(name, opts?)` → queue — Get task queue info. `opts`: `{namespace, task_queue_type}`
- `c:schedules(opts?)` → `{schedules}` — List schedules. `opts`: `{namespace, maximum_page_size}`
- `c:schedule(schedule_id, opts?)` → schedule — Get schedule by ID. `opts`: `{namespace}`
- `c:search(query, opts?)` → `{executions}` — Search workflows by visibility query. `opts`: `{namespace, page_size}`
- `c:is_workflow_running(workflow_id, opts?)` → bool — Check if workflow status is RUNNING
- `c:wait_workflow_complete(workflow_id, timeout_secs, opts?)` → workflow — Wait for workflow completion, errors on timeout

```lua
local temporal = require("assay.temporal")
local c = temporal.client("http://temporal:7233", {namespace = "my-namespace"})
local running = c:is_workflow_running("my-workflow-id")
if running then
  c:signal_workflow("my-workflow-id", "approve", {approved = true})
end
```

---

## temporal.connect / temporal.start (gRPC client)

Native gRPC client for starting and interacting with workflows.
No `require` needed — the `temporal` global is registered automatically.

### Connection

```lua
-- Persistent client (reuses gRPC connection)
local client = temporal.connect({
  url = "temporal-frontend:7233",  -- host:port, no http://
  namespace = "my-namespace",      -- optional, defaults to "default"
})

-- One-shot convenience (creates new connection each call)
local handle = temporal.start({
  url = "temporal-frontend:7233",
  namespace = "my-namespace",
  task_queue = "my-queue",
  workflow_type = "ProcessOrder",
  workflow_id = "order-12345",
  input = { item = "widget" },
})
```

### Client Methods

- `client:start_workflow({ task_queue, workflow_type, workflow_id, input? })` — returns `{workflow_id, run_id}`
- `client:signal_workflow({ workflow_id, signal_name, input? })` — send signal to running workflow
- `client:query_workflow({ workflow_id, query_type, input? })` — query workflow state, returns decoded JSON
- `client:describe_workflow(workflow_id)` — returns `{status, workflow_type, run_id, start_time, close_time, history_length}`
- `client:get_result({ workflow_id, follow_runs? })` — blocks until workflow completes, returns result
- `client:cancel_workflow(workflow_id)` — request graceful cancellation
- `client:terminate_workflow(workflow_id)` — force terminate

Status values: RUNNING, COMPLETED, FAILED, CANCELED, TERMINATED, CONTINUED_AS_NEW, TIMED_OUT.

---

## temporal.worker (Worker runtime)

Start a worker that polls Temporal for activity and workflow tasks, executes them
as Lua functions, and reports results back.

```lua
local handle = temporal.worker({
  url = "temporal-frontend:7233",
  namespace = "my-namespace",       -- optional, defaults to "default"
  task_queue = "promotions",

  activities = {
    update_gitops = function(input)
      local resp = http.post(input.gitlab_url, input.commit, {
        headers = { ["PRIVATE-TOKEN"] = input.token },
      })
      if resp.status ~= 201 then error("GitLab commit failed: HTTP " .. resp.status) end
      return json.parse(resp.body)
    end,

    notify = function(input)
      http.post(input.webhook, { text = input.message })
      return { sent = true }
    end,
  },

  workflows = {
    PromotionWorkflow = function(ctx, input)
      -- Stage 1: wait for human approval (or timeout after 24h)
      local approval = ctx:wait_signal("approve", { timeout = 86400 })
      if not approval then
        return { status = "timed_out" }
      end

      -- Stage 2: update GitOps overlays
      local commit = ctx:execute_activity("update_gitops", {
        gitlab_url = input.gitlab_url,
        commit = input.gitops_commit,
        token = input.token,
      }, { start_to_close_timeout = 30, retry_policy = { maximum_attempts = 3 } })

      -- Stage 3: notify
      ctx:execute_activity("notify", {
        webhook = input.webhook,
        message = "Deployed " .. input.version .. " to " .. input.target,
      })

      return { status = "done", commit_id = commit.short_id, approved_by = approval.user }
    end,
  },
})

-- handle:is_running()  → true while worker is active
-- handle:shutdown()    → graceful shutdown, drains in-flight tasks
```

### Activities

Activities are plain Lua functions that perform real I/O (HTTP calls, database queries, etc.).
They receive a single `input` value (deserialized from JSON) and return a result (serialized to JSON).
Temporal retries failed activities according to the retry policy.

### Workflows

Workflows are Lua functions that receive `(ctx, input)` and orchestrate activities.
Each workflow runs as a Lua coroutine. The `ctx` object provides deterministic methods —
on replay after a worker restart, `ctx` methods return cached results from history
instead of re-executing, so the workflow fast-forwards to the correct point.

### ctx:execute_activity(name, input, opts?)

Schedule a registered activity and block until it completes. Returns the activity's result.
Throws a Lua `error()` if the activity fails after retries are exhausted.

```lua
local result = ctx:execute_activity("update_gitops", {
  target = "prod",
  version = "v0.2.0",
}, {
  start_to_close_timeout = 300,     -- seconds (default: 300)
  schedule_to_close_timeout = 600,  -- overall deadline including queue time
  heartbeat_timeout = 30,           -- activity must heartbeat within this interval
  retry_policy = {
    initial_interval = 1,           -- seconds between retries
    backoff_coefficient = 2.0,      -- exponential backoff multiplier
    maximum_interval = 60,          -- cap on retry interval
    maximum_attempts = 5,           -- 0 = unlimited
    non_retryable_errors = { "PERMISSION_DENIED" },
  },
})
```

### ctx:wait_signal(name, opts?)

Block until an external signal is received or timeout expires.
Returns the signal payload, or `nil` on timeout.

```lua
-- Block indefinitely
local payload = ctx:wait_signal("approve")

-- Block with timeout (returns nil on timeout)
local payload = ctx:wait_signal("approve", { timeout = 86400 })
-- payload = { user = "jane" }  (whatever the signaller sent)
```

Signals are buffered — if a signal arrives before `wait_signal` is called,
it is delivered immediately.

Send a signal from outside (gRPC client):
```lua
client:signal_workflow({ workflow_id = "promote-v0.2.0", signal_name = "approve", input = { user = "jane" } })
```

### ctx:sleep(seconds)

Deterministic sleep using a Temporal timer. On replay, returns immediately
if the timer already fired in history.

```lua
ctx:sleep(60)  -- wait 1 minute (Temporal timer, not wall clock)
```

### ctx:side_effect(fn)

Run a non-deterministic function. Use for generating IDs or reading wall-clock time
inside a workflow. For truly non-deterministic operations (external API calls),
use an activity instead.

```lua
local id = ctx:side_effect(function() return crypto.random(16) end)
local now = ctx:side_effect(function() return time() end)
```

### ctx:workflow_info()

Returns metadata about the current workflow execution.

```lua
local info = ctx:workflow_info()
-- {
--   workflow_id = "promote-v0.2.0-to-prod",
--   workflow_type = "PromotionWorkflow",
--   namespace = "command-center",
--   task_queue = "promotions",
--   attempt = 1,
--   start_time = 1712851200.0,
-- }
```

### Starting a workflow (from the same or another app)

```lua
-- Connect to Temporal
temporal.connect({ url = "temporal-frontend:7233", namespace = "command-center" })

-- Start the workflow
local run = temporal.start_workflow({
  task_queue = "promotions",
  workflow_type = "PromotionWorkflow",
  workflow_id = "promote-v0.2.0-to-prod",
  input = {
    version = "v0.2.0",
    target = "prod",
    gitlab_url = "https://gitlab.example.com/api/v4/projects/123/repository/commits",
    token = env.get("GITLAB_TOKEN"),
    webhook = "https://hooks.slack.com/...",
  },
})

-- Later: approve via signal
client:signal_workflow({ workflow_id = "promote-v0.2.0-to-prod", signal_name = "approve", input = { user = "jane" } })

-- Wait for result
local result = client:get_result({ workflow_id = "promote-v0.2.0-to-prod" })
```
