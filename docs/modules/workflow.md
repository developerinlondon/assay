## workflow

Durable workflow engine and Lua client. The engine runs in `assay serve`; any assay Lua app becomes
a worker via `require("assay.workflow")`. Workflows are event-sourced and crash-safe — activities
are retried on failure and their results are persisted before progress continues.

Three pieces, one binary:

- **Engine** — `assay serve` starts a long-lived server (REST + SSE + dashboard).
- **CLI** — `assay workflow` and `assay schedule` manage workflows from the shell.
- **Client** — `require("assay.workflow")` lets Lua apps register activities and start/inspect
  workflows.

The engine and clients communicate over HTTP — any language with an HTTP client can be a worker, not
just Lua.

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
  Okta, Dex, Keycloak, Cloudflare Access, ...).

SQLite is single-instance only — the engine takes a `engine_lock` row and refuses to start if
another instance holds it. For multi-instance deployment (Kubernetes, Docker Swarm), use Postgres;
the cron scheduler picks a leader via `pg_advisory_lock` and only the leader fires schedules.

The engine serves:

- `GET  /api/v1/health` — liveness probe
- `GET  /api/v1/openapi.json` — OpenAPI 3 spec for all endpoints
- `GET  /api/v1/docs` — interactive API docs (Scalar)
- `GET  /workflow/` — built-in dashboard (workflows, schedules, workers, queues, namespaces,
  settings; live updates over SSE; light + dark theme)
- `GET  /api/v1/events/stream?namespace=X` — SSE event stream
- 21+ REST endpoints for workflow lifecycle, worker registration, task polling, schedules,
  namespaces — see the OpenAPI spec for the full list

### CLI — `assay workflow` / `assay schedule`

Talk to a running engine. Reads `ASSAY_ENGINE_URL` (default `http://localhost:8080`).

- `assay workflow list [--status RUNNING] [--type IngestData]` — list workflows
- `assay workflow describe <workflow-id>` — full state, history, children
- `assay workflow signal <workflow-id> <signal-name> [payload]` — send signal
- `assay workflow cancel <workflow-id>` — graceful cancel (workflow can clean up)
- `assay workflow terminate <workflow-id> [--reason "..."]` — hard stop
- `assay schedule list` — list cron schedules
- `assay schedule create <name> --type IngestData --cron "0 * * * *"` — new schedule
- `assay schedule pause <name>` — stop firing (kept on disk)
- `assay schedule resume <name>` — re-enable
- `assay schedule delete <name>` — remove

### Lua client — `require("assay.workflow")`

Register the assay process as a worker that picks up activity tasks from the engine. The workflow
handlers themselves run server-side — the Lua client provides activity implementations and
inspection.

- `workflow.connect(url, opts?)` → nil — Connect to the engine and verify reachability
  - `url`: engine URL (e.g. `"http://localhost:8080"`)
  - `opts`: `{ token = "Bearer abc..." }` for auth (api-key or JWT)
- `workflow.activity(name, handler)` → nil — Register an activity implementation
  - `handler(ctx, input) -> result` — `result` is JSON-serialised and persisted
  - Errors raised inside the handler mark the activity failed; the engine retries per the activity's
    retry policy
- `workflow.define(name, handler)` → nil — Register a workflow type (handler body is reference-only;
  workflow execution is server-driven via the event log)
- `workflow.listen(opts)` → blocks — Register as a worker and start polling
  - `opts.queue` (default `"default"`) — task queue to poll
  - `opts.identity` — friendly worker name (default `"assay-worker-<hostname>"`)
  - `opts.max_concurrent_workflows` (default 10), `opts.max_concurrent_activities` (default 20)

Client-side inspection / control (no `listen` required):

- `workflow.start(opts)` → `{ workflow_id, run_id, status }` — Start a workflow
  - `opts`: `{ workflow_type = "...", workflow_id = "...", input = {...}, task_queue = "..." }`
- `workflow.signal(workflow_id, signal_name, payload)` → nil — Send a signal
- `workflow.describe(workflow_id)` → `{ status, type, namespace, queue, created_at, ... }` — Get
  state
- `workflow.cancel(workflow_id)` → nil — Cancel a running workflow

### Example

```lua
-- worker.lua — runs as a worker
local workflow = require("assay.workflow")

workflow.connect("http://assay.example.com", { token = os.getenv("ASSAY_TOKEN") })

workflow.activity("fetch_s3", function(ctx, input)
  local resp = http.get("https://s3.amazonaws.com/" .. input.bucket .. "/" .. input.key)
  if resp.status ~= 200 then error("fetch failed: " .. resp.status) end
  return { bytes = #resp.body, body = resp.body }
end)

workflow.activity("load_warehouse", function(ctx, input)
  local conn = db.connect(os.getenv("WAREHOUSE_URL"))
  db.execute(conn, "INSERT INTO ingest (data) VALUES ($1)", { input.body })
  db.close(conn)
  return { rows_loaded = 1 }
end)

workflow.listen({ queue = "data-pipeline" })  -- blocks
```

```lua
-- starter.lua — kicks off a workflow from any assay app
local workflow = require("assay.workflow")
workflow.connect("http://assay.example.com")

local wf = workflow.start({
  workflow_type = "IngestData",
  workflow_id   = "ingest-" .. os.time(),
  task_queue    = "data-pipeline",
  input         = { bucket = "data-lake", key = "batch-45.parquet" },
})

print("started", wf.workflow_id, wf.status)
```

### Concepts

- **Activity** — a unit of work with at-least-once semantics. The engine records the result before
  progress continues. Activities have configurable retry policy, start-to-close timeout, and
  heartbeat timeout.
- **Workflow** — a deterministic orchestration of activities, sleeps, signals, and child workflows.
  The full event history is persisted, so a crashed engine can replay state on restart.
- **Task queue** — a named queue that workers subscribe to. Workflows are routed to queues; only
  workers on that queue pick them up. Use queues to isolate workloads (e.g. `gpu-tasks`,
  `data-pipeline`, `default`).
- **Namespace** — a logical tenant. Workflows, schedules, and workers in one namespace are invisible
  to others. Default namespace is `main`. Manage via the dashboard or `POST /api/v1/namespaces`.
- **Signal** — an asynchronous message delivered to a running workflow. Used for human-in-the-loop
  steps, pausing, or external events.
- **Schedule** — a cron expression that starts a workflow on a recurring basis. The scheduler uses
  leader election under Postgres so only one engine fires.
- **Child workflow** — a workflow started by another workflow. Cancellation propagates from parent
  to all children recursively.
- **Continue-as-new** — a workflow restarts itself with a fresh history, used for very long-running
  loops to keep the event log bounded.

### Notes

- `workflow.connect` does a `GET /api/v1/health` and errors loudly if the engine is unreachable —
  surfaces config mistakes early instead of at first use.
- Activities run inside `pcall`. Any `error()` raised inside a handler is reported back to the
  engine as a failure, and the engine applies the activity's retry policy.
- The engine is a separate crate (`assay-workflow`) — it's also publishable to crates.io for
  embedding in non-Lua Rust applications.
- The whole engine + dashboard + client is gated behind the `workflow` feature flag, which is
  enabled by default. To build without it:
  `cargo install assay-lua --no-default-features --features cli,db,server`. When disabled,
  `assay serve` prints an error instead of starting.
