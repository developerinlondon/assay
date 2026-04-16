## workflow

Durable workflow engine + Lua client. The engine runs in `assay serve`; any assay Lua app becomes a
worker via `require("assay.workflow")`. Workflow code runs deterministically and replays from a
persisted event log, so worker crashes don't lose work and side effects don't duplicate.

Three pieces, one binary:

- **Engine** — `assay serve` starts a long-lived server (REST + SSE + dashboard).
- **CLI** — `assay workflow` and `assay schedule` manage workflows from the shell.
- **Client** — `require("assay.workflow")` lets Lua apps register activities + workflow handlers and
  become workers.

The engine and clients communicate over HTTP — any language with an HTTP client can implement a
worker, not just Lua.

### Engine — `assay serve`

Start the workflow server.

- `assay serve` — start with default SQLite backend, port 8080, no auth
- `assay serve --port 8085` — listen on a different port
- `assay serve --backend sqlite:///var/lib/assay/workflows.db` — explicit SQLite path
- `assay serve --backend postgres://user:pass@host:5432/assay` — Postgres for multi-instance
- `DATABASE_URL=postgres://...  assay serve` — read backend URL from env (avoids putting credentials
  in argv, where they'd show up in `ps`)

Authentication modes (mutually exclusive — pick one):

- `--no-auth` (default) — open access. Use only behind a trusted gateway.
- `--auth-api-key` — clients send `Authorization: Bearer <key>`. Manage keys with
  `--generate-api-key` and `--list-api-keys`. Keys are SHA256-hashed at rest.
- `--auth-issuer https://idp.example.com --auth-audience assay` — JWT/OIDC. The engine fetches and
  caches the issuer's JWKS to validate signatures; works with any standard OIDC provider (Auth0,
  Okta, Dex, Keycloak, Cloudflare Access, …).

SQLite is single-instance only — the engine takes an `engine_lock` row and refuses to start if
another instance holds it. For multi-instance deployment (Kubernetes, Docker Swarm), use Postgres;
the cron scheduler picks a leader via `pg_advisory_lock` so only one engine fires.

The engine serves:

- `GET  /api/v1/health` — liveness probe
- `GET  /api/v1/openapi.json` — OpenAPI 3 spec for all endpoints
- `GET  /api/v1/docs` — interactive API docs (Scalar)
- `GET  /workflow/` — built-in dashboard (workflows, schedules, workers, queues, namespaces,
  settings; live updates over SSE; light + dark theme)
- `GET  /api/v1/events/stream?namespace=X` — SSE event stream
- 23+ REST endpoints for workflow lifecycle, worker registration, task polling, schedules,
  namespaces, workflow-task dispatch — see the OpenAPI spec for the full list

`ASSAY_WF_DISPATCH_TIMEOUT_SECS` env var (default `30`) controls how long a worker can be silent
before its dispatch lease is forcibly released — see "crash safety" below.

### CLI — `assay workflow` / `assay schedule`

Talk to a running engine. Reads `ASSAY_ENGINE_URL` (default `http://localhost:8080`).

- `assay workflow list [--status RUNNING] [--type IngestData]`
- `assay workflow describe <workflow-id>` — full state, history, children
- `assay workflow signal <workflow-id> <signal-name> [payload]`
- `assay workflow cancel <workflow-id>` — graceful cancel (workflow gets a chance to clean up)
- `assay workflow terminate <workflow-id> [--reason "…"]` — hard stop
- `assay schedule list`
- `assay schedule create <name> --type IngestData --cron "0 * * * * *"` — 6-field cron (with
  seconds)
- `assay schedule pause <name>` / `resume <name>` / `delete <name>`

### Lua client — `require("assay.workflow")`

Register the assay process as a worker that runs **both** workflow handlers (orchestration) and
activity handlers (concrete work) for a queue.

- `workflow.connect(url, opts?)` → nil — Connect and verify the engine is reachable
  - `url`: engine URL (e.g. `"http://localhost:8080"`)
  - `opts`: `{ token = "Bearer abc..." }` for auth (api-key or JWT)
- `workflow.define(name, handler)` → nil — Register a workflow type. Handler runs as a coroutine;
  uses `ctx:` methods to drive activities, timers, signals, child workflows. See "Workflow handler
  context" below.
- `workflow.activity(name, handler)` → nil — Register an activity implementation. Activities run
  once and their result is persisted; failures retry per the activity's policy.
- `workflow.listen(opts)` → blocks — Polls workflow tasks AND activity tasks on the queue.
  - `opts.queue` (default `"default"`) — task queue
  - `opts.identity` — friendly worker name (default `"assay-worker-<hostname>"`)
  - `opts.max_concurrent_workflows` (default 10), `opts.max_concurrent_activities` (default 20)

Client-side inspection / control (no `listen` required):

- `workflow.start(opts)` → `{ workflow_id, run_id, status }` — Start a workflow
  - `opts`: `{ workflow_type, workflow_id, input?, task_queue? }`
- `workflow.signal(workflow_id, signal_name, payload)` — Send a signal
- `workflow.describe(workflow_id)` → table — Get current state + result
- `workflow.cancel(workflow_id)` — Cancel a running workflow

### Workflow handler context (`ctx`)

Inside `workflow.define(name, function(ctx, input) ... end)`:

- `ctx:execute_activity(name, input, opts?)` → result — Schedule an activity, block until complete,
  return result. Raises if the activity fails after retries. `opts`:
  `{ task_queue?, max_attempts?, initial_interval_secs?, backoff_coefficient?,
  start_to_close_secs?, heartbeat_timeout_secs? }`.
- `ctx:sleep(seconds)` → nil — Durable timer. Survives worker bouncing; another worker resumes the
  workflow when the timer fires.
- `ctx:wait_for_signal(name)` → payload — Block until a matching signal arrives. Returns the
  signal's JSON payload (or nil if signaled with no payload). Multiple waits for the same name
  consume signals in arrival order.
- `ctx:start_child_workflow(workflow_type, opts)` → result — Start a child workflow and block until
  it completes; raises if it failed. `opts.workflow_id` is required and **must be deterministic**
  (same id every replay).
- `ctx:side_effect(name, function() … end)` → value — Run a non-deterministic operation exactly
  once. The function runs in the worker, the value is recorded in the workflow event log, and on
  every subsequent replay the cached value is returned without re-running. Use for `crypto.uuid()`,
  `os.time()`, anything reading external mutable state.

Inside `workflow.activity(name, function(ctx, input) ... end)`:

- `ctx:heartbeat(details?)` — Tell the engine you're still alive. Required for activities with
  `heartbeat_timeout_secs` set; the engine reassigns the activity if heartbeats stop.

### Crash safety

Workflow code is **deterministic by replay**. Each `ctx:` call gets a per-execution sequence number
and the engine persists `ActivityScheduled/Completed/Failed`, `TimerScheduled/Fired`,
`SignalReceived`, `SideEffectRecorded`, `ChildWorkflowStarted/Completed/Failed`,
`WorkflowAwaitingSignal`, `WorkflowCancelRequested` events. When a worker is asked to run a workflow
task it receives the full event history; `ctx:` calls short-circuit to cached values for everything
that's already in history, so the workflow always reaches the same state and the only thing that
re-runs is the next unfulfilled step.

Specific crash modes:

- **Activity worker dies mid-execution** — the activity's `last_heartbeat` ages out (per-activity
  `heartbeat_timeout_secs`); the engine re-queues per the retry policy.
- **Workflow worker dies mid-replay** — the workflow's `dispatch_last_heartbeat` ages out
  (`ASSAY_WF_DISPATCH_TIMEOUT_SECS`, default 30s); any worker on the queue picks it up and replays
  from the event log.
- **Engine dies** — all state is in the DB. On restart, in-flight workflow + activity tasks become
  claimable again as their heartbeats age out.

`ctx:side_effect` is the escape hatch for any operation that would produce different values across
replays (current time, random IDs, external HTTP). The result is recorded once on first execution
and returned from cache thereafter, even after a worker crash.

### Example — sequential activities + signal

```lua
local workflow = require("assay.workflow")
workflow.connect("http://assay.example.com", { token = env.get("ASSAY_TOKEN") })

workflow.define("ApproveAndDeploy", function(ctx, input)
  local artifact = ctx:execute_activity("build", { ref = input.git_sha })
  -- pause until a human signals "approve" via the API or dashboard
  local approval = ctx:wait_for_signal("approve")
  return ctx:execute_activity("deploy", {
    image = artifact.image,
    env = input.target_env,
    approver = approval.by,
  })
end)

workflow.activity("build", function(ctx, input)
  local resp = http.post("https://ci/build", { ref = input.ref })
  if resp.status ~= 200 then error("build failed: " .. resp.status) end
  return { image = json.parse(resp.body).image }
end)

workflow.activity("deploy", function(ctx, input)
  local resp = http.post("https://k8s/apply", input)
  if resp.status ~= 200 then error("deploy failed: " .. resp.status) end
  return { url = json.parse(resp.body).url, approver = input.approver }
end)

workflow.listen({ queue = "deploys" })  -- blocks
```

Start a run, signal approval, see the result:

```sh
assay workflow start --type ApproveAndDeploy --id deploy-1234 \
  --input '{"git_sha":"abc123","target_env":"staging"}'

assay workflow signal deploy-1234 approve '{"by":"alice"}'

assay workflow describe deploy-1234   # status: COMPLETED, result: {url, approver}
```

### Concepts

- **Activity** — a unit of work with at-least-once semantics. Result persisted before progress
  continues. Configurable retry policy, start-to-close timeout, heartbeat timeout.
- **Workflow** — deterministic orchestration of activities, sleeps, signals, child workflows. Full
  event history is persisted; a crashed worker → another worker replays from history.
- **Task queue** — a named queue workers subscribe to. Workflows are routed to a queue; only workers
  on that queue claim them.
- **Namespace** — logical tenant. Workflows / schedules / workers in one namespace are invisible to
  others. Default `main`. Manage via the dashboard or `POST /api/v1/namespaces`.
- **Signal** — async message delivered to a running workflow; consumed via `ctx:wait_for_signal`.
- **Schedule** — cron expression that starts a workflow recurringly. The engine's scheduler uses
  leader election under Postgres so only one instance fires.
- **Child workflow** — workflow started by another workflow. Cancellation propagates from parent to
  all children recursively.
- **Side effect** — non-deterministic operation captured in history on first call so all replays see
  the same value.

### Dashboard

`/workflow/` (or just `/` — redirects). Real-time monitoring, dark/light, brand-aligned with
[assay.rs](https://assay.rs). Views: workflows (list with status filter, drill-in to event
timeline + children), schedules, workers, queues, namespaces, settings. Live updates via SSE.
Cache-busted asset URLs (per-process startup stamp) so a deploy is reflected immediately.

### Notes

- The whole engine + dashboard + Lua client is gated behind the `workflow` cargo feature, which is
  **enabled by default**. To build assay without the engine:
  `cargo install assay-lua --no-default-features --features cli,db,server`. When disabled,
  `assay serve` prints an error instead of starting.
- The cron crate used by the scheduler requires **6- or 7-field** cron expressions (with seconds).
  The common 5-field form fails to parse. Use `0 * * * * *` for "every minute on the zero second" or
  `* * * * * *` for "every second."
- Parallel activities (Promise.all-style) are not yet supported. Use sequential
  `ctx:execute_activity` calls or kick off independent child workflows. Tracked as a follow-up.
- The engine is also publishable as a standalone Rust crate (`assay-workflow`) for embedding in
  non-Lua Rust applications.
