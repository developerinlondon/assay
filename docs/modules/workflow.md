## workflow

Durable workflow engine + Lua client. The engine runs in `assay serve`; any assay Lua app becomes a
worker via `require("assay.workflow")`. Workflow code runs deterministically and replays from a
persisted event log, so worker crashes don't lose work and side effects don't duplicate.

Four pieces, one binary:

```
┌──────────────────────────────────────────────────────────────────────┐
│ assay serve              the engine (REST + SSE + dashboard)         │
│ assay <subcommand>       CLI (workflow / schedule / namespace /      │
│                                worker / queue / completion)          │
│ require("assay.workflow") Lua client: handlers + management surface  │
│ REST API + OpenAPI spec  any-language workers via openapi-generator  │
└──────────────────────────────────────────────────────────────────────┘
```

The engine and clients communicate over HTTP — any language with an HTTP client can implement a
worker or management script, not just Lua.

### Engine — `assay serve`

Start the workflow server.

```sh
assay serve                                           # SQLite, port 8080, no auth (dev)
assay serve --port 8085                               # different port
assay serve --backend sqlite:///var/lib/assay/w.db    # explicit SQLite path
assay serve --backend postgres://u:p@h:5432/assay     # Postgres (multi-instance)
DATABASE_URL=postgres://... assay serve               # backend from env (keeps creds out of argv)
```

Auth modes:

| Flag                                        | Behaviour                                                                                                                                         |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--no-auth` (default)                       | Open access. Use only behind a trusted gateway.                                                                                                   |
| `--auth-api-key`                            | Clients send `Authorization: Bearer <key>`. Manage keys with `--generate-api-key` / `--list-api-keys`. Keys are SHA256-hashed at rest.            |
| `--auth-issuer <url> --auth-audience <aud>` | JWT/OIDC. Fetches and caches the issuer's JWKS to validate signatures. Works with Auth0, Okta, Dex, Keycloak, Cloudflare Access, any OIDC issuer. |
| `--auth-api-key` + `--auth-issuer …`        | Combined. Tokens that parse as a JWS header take the JWT path; everything else takes the API-key path. Same server accepts both token types on `Authorization: Bearer`. |

**Combined mode dispatch** (when both `--auth-issuer` and `--auth-api-key` are set):

- `Authorization: Bearer <jwt>` — validated against JWKS. Rejected if expired / wrong
  issuer / wrong audience / bad signature. A semantically-invalid JWT is **not** silently
  retried as an API key.
- `Authorization: Bearer <api-key>` — hashed and looked up in the store.
- `Authorization: Bearer <garbage>` — 401.

Combined mode lets the same server serve short-lived OIDC user tokens (from a browser
session) alongside long-lived machine API keys (from a CI job) without the caller picking
a mode up front.

**Multi-instance deployment.** SQLite is single-instance only (engine takes an `engine_lock` row at
startup). For Kubernetes / Docker Swarm, use Postgres: the cron scheduler picks a leader via
`pg_advisory_lock` so only one instance fires; workflow + activity task claiming uses
`FOR UPDATE SKIP LOCKED` so multiple instances don't race.

**Optional S3 archival** (cargo feature `s3-archival`, default-off). When compiled in and
`ASSAY_ARCHIVE_S3_BUCKET` is set at runtime, a background task periodically finds workflows in
terminal states older than `ASSAY_ARCHIVE_RETENTION_DAYS` (default 30), uploads `{record, events}`
to `s3://bucket/prefix/<namespace>/<workflow_id>.json`, and purges dependent rows. The workflow stub
stays with `archived_at` + `archive_uri` set so `GET /workflows/{id}` still resolves. Credentials
resolve via the AWS SDK default chain (env / shared config / IRSA).

| Env var                          | Default  | Meaning                                                       |
| -------------------------------- | -------- | ------------------------------------------------------------- |
| `ASSAY_ARCHIVE_S3_BUCKET`        | (unset)  | Enables archival when set                                     |
| `ASSAY_ARCHIVE_S3_PREFIX`        | `assay/` | S3 key prefix                                                 |
| `ASSAY_ARCHIVE_RETENTION_DAYS`   | 30       | Min age before archiving                                      |
| `ASSAY_ARCHIVE_POLL_SECS`        | 3600     | How often the archiver runs                                   |
| `ASSAY_ARCHIVE_BATCH_SIZE`       | 50       | Max workflows archived / tick                                 |
| `ASSAY_WF_DISPATCH_TIMEOUT_SECS` | 30       | Worker silent-timeout for dispatch lease (see "crash safety") |

The engine serves:

| Path                        | Purpose                                     |
| --------------------------- | ------------------------------------------- |
| `GET /api/v1/health`        | Liveness probe                              |
| `GET /api/v1/version`       | `{ version, build_profile }` — CLI + UI use |
| `GET /api/v1/openapi.json`  | Full OpenAPI 3 spec (all ~30 endpoints)     |
| `GET /api/v1/docs`          | Interactive API docs (Scalar)               |
| `GET /workflow/`            | Built-in dashboard (see "Dashboard" below)  |
| `GET /api/v1/events/stream` | SSE event stream                            |

Full endpoint list in the OpenAPI spec — workflow lifecycle, state queries, events, children,
continue-as-new, signals, schedules (CRUD + patch/pause/resume), namespaces, workers, queues, worker
task polling and dispatch.

### CLI

Talk to a running engine from a shell. Lua stdlib (below) is the preferred path for automation; CLI
is for operators at a terminal and one-shot shell scripts.

**Global options** — flag / env / config file / default precedence:

| Flag               | Env var              | Config key     | Default                           |
| ------------------ | -------------------- | -------------- | --------------------------------- |
| `--engine-url URL` | `ASSAY_ENGINE_URL`   | `engine_url`   | `http://127.0.0.1:8080`           |
| `--api-key KEY`    | `ASSAY_API_KEY`      | `api_key`      | (none)                            |
| (via config only)  | `ASSAY_API_KEY_FILE` | `api_key_file` | (none; read + trim file contents) |
| `--namespace NS`   | `ASSAY_NAMESPACE`    | `namespace`    | `main`                            |
| `--output FORMAT`  | `ASSAY_OUTPUT`       | `output`       | `table` on TTY, `json` when piped |
| `--config PATH`    | `ASSAY_CONFIG_FILE`  | (n/a)          | see discovery order               |

**Config file** — YAML, auto-discovered at (first match wins):

1. `--config PATH` (explicit)
2. `$ASSAY_CONFIG_FILE`
3. `$XDG_CONFIG_HOME/assay/config.yaml`
4. `~/.config/assay/config.yaml`
5. `/etc/assay/config.yaml`

```yaml
engine_url: https://assay.example.com
api_key_file: /run/secrets/assay-api-key # preferred — keeps the secret out of env / argv
namespace: main
output: table
```

`api_key_file` wins over `api_key`. Missing file is not an error — callers fall through to flag /
env / default precedence.

**JSON input indirection.** `--input`, `--search-attrs`, and signal payloads all accept:

```
'{"key":"value"}'       # literal
@/path/to/file.json     # file contents
-                       # read stdin
```

**Subcommand surface:**

```
assay workflow
  start     --type T [--id ID] [--input JSON] [--queue Q] [--search-attrs JSON]
  list      [--status S] [--type T] [--search-attrs JSON] [--limit N]
  describe  <id>
  state     <id> [<query-name>]                 # register_query reader
  events    <id> [--follow]                     # log, or poll-stream until terminal
  children  <id>
  signal    <id> <name> [payload]
  cancel    <id>
  terminate <id> [--reason R]
  continue-as-new <id> [--input JSON]           # client-side (distinct from ctx:)
  wait      <id> [--timeout SECS] [--target STATUS]   # scripting-friendly blocking

assay schedule
  list  |  describe <name>
  create <name> --type T --cron EXPR [--timezone TZ] [--input JSON] [--queue Q]
  patch  <name> [--cron EXPR] [--timezone TZ] [--input JSON] [--queue Q] [--overlap POLICY]
  pause  <name>  |  resume <name>  |  delete <name>

assay namespace  create | list | describe | delete
assay worker     list
assay queue      stats
assay completion  <bash|zsh|fish|powershell|elvish>
```

**Exit codes:** 0 success · 1 HTTP / unreachable / not-found · 2 `workflow wait` timeout · 64 usage
error (bad JSON).

**Shell completion:**

```sh
assay completion bash > /etc/bash_completion.d/assay
assay completion zsh  > "${fpath[1]}/_assay"
assay completion fish > ~/.config/fish/completions/assay.fish
```

### Lua client — `require("assay.workflow")`

Two roles in one module: **worker** (register handlers and block polling for tasks) and
**management** (inspect / mutate the engine from anywhere, same as the CLI).

#### Worker role

- `workflow.connect(url, opts?)` → nil — `opts`: `{ token = "<api-key-or-jwt>" }`
- `workflow.define(name, handler)` → nil — register a workflow type
- `workflow.activity(name, handler)` → nil — register an activity
- `workflow.listen(opts)` → blocks — poll workflow + activity tasks on a queue
  - `opts.queue` (default `"default"`), `opts.identity`, `opts.max_concurrent_workflows` (10),
    `opts.max_concurrent_activities` (20)

#### Management role (new in v0.11.3 — parity with REST)

**Workflows:**

| Function                               | REST                                   |
| -------------------------------------- | -------------------------------------- |
| `workflow.start(opts)`                 | `POST /workflows`                      |
| `workflow.list(opts?)`                 | `GET  /workflows?...`                  |
| `workflow.describe(id)`                | `GET  /workflows/{id}`                 |
| `workflow.get_events(id)`              | `GET  /workflows/{id}/events`          |
| `workflow.get_state(id, name?)`        | `GET  /workflows/{id}/state[/{name}]`  |
| `workflow.list_children(id)`           | `GET  /workflows/{id}/children`        |
| `workflow.signal(id, name, payload)`   | `POST /workflows/{id}/signal/{name}`   |
| `workflow.cancel(id)`                  | `POST /workflows/{id}/cancel`          |
| `workflow.terminate(id, reason?)`      | `POST /workflows/{id}/terminate`       |
| `workflow.continue_as_new(id, input?)` | `POST /workflows/{id}/continue-as-new` |

`workflow.list(opts)` accepts `{ namespace?, status?, type?, search_attrs?, limit?, offset? }`.
`search_attrs` is a table; the CLI URL-encodes it as the `search_attrs=` query param.

**Sub-tables** (one per REST resource):

- `workflow.schedules.{create, list, describe, patch, pause, resume, delete}`
- `workflow.namespaces.{create, list, describe, stats, delete}`
- `workflow.workers.list(opts?)`
- `workflow.queues.stats(opts?)`

Every function returns the parsed JSON response on success, `nil` on a 404 for
`describe`/`get_state`, or raises `error()` with an HTTP status message otherwise — consistent with
the existing `workflow.start / signal / describe / cancel` behaviour.

### Workflow handler context (`ctx`)

Inside `workflow.define(name, function(ctx, input) ... end)`:

| Method                                          | Behaviour                                                                                                                                                              |
| ----------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ctx:execute_activity(name, input, opts?)`      | Schedule an activity, block until complete, return result. Raises on final failure. `opts`: retry + timeout knobs (see below).                                         |
| `ctx:execute_parallel(activities)`              | **v0.11.3.** Schedule N activities concurrently, return results in input order. Raises if any fail. Handler resumes only when all have terminal events.                |
| `ctx:sleep(seconds)`                            | Durable timer. Survives worker bouncing; another worker resumes when due.                                                                                              |
| `ctx:wait_for_signal(name)` → payload           | Block until a matching signal arrives. Payload is the signal's JSON value (or nil if signaled with no payload). Multiple waits consume in order.                       |
| `ctx:start_child_workflow(workflow_type, opts)` | Start a child, block until it completes. `opts.workflow_id` is required and **must be deterministic** (same id every replay).                                          |
| `ctx:side_effect(name, fn)`                     | Run a non-deterministic op exactly once. Value is cached in history; all replays return the cached value.                                                              |
| `ctx:register_query(name, fn)`                  | **v0.11.3.** Expose live workflow state to external callers via `GET /workflows/{id}/state[/{name}]`. Handler runs on every replay; result is persisted as a snapshot. |
| `ctx:upsert_search_attributes(patch)`           | **v0.11.3.** Merge a table into the workflow's `search_attributes` so callers can filter on it via `workflow.list({ search_attrs = ... })`.                            |
| `ctx:continue_as_new(input)`                    | **v0.11.3.** Close this run and start a fresh one with empty history (same type / namespace / queue). Standard pattern for unbounded-loop workflows.                   |

`opts` on `execute_activity` / `execute_parallel`:
`{ task_queue?, max_attempts?, initial_interval_secs?, backoff_coefficient?, start_to_close_secs?,
heartbeat_timeout_secs? }`.

Inside `workflow.activity(name, function(ctx, input) ... end)`:

- `ctx:heartbeat(details?)` — required for activities with `heartbeat_timeout_secs`; the engine
  reassigns the activity if heartbeats stop.

### Crash safety

Workflow code is **deterministic by replay**. Each `ctx:` call gets a per-execution sequence number
and the engine persists every completed command as an event:

```
ActivityScheduled / Completed / Failed
TimerScheduled / Fired
SignalReceived                            WorkflowStarted / Completed / Failed / Cancelled
SideEffectRecorded                        WorkflowAwaitingSignal / CancelRequested
ChildWorkflowStarted / Completed / Failed
```

When a worker picks up a workflow task it receives the full event history. `ctx:` calls
short-circuit to cached values for everything already in history, so the workflow always reaches the
same state and only the next unfulfilled step actually runs.

| Failure mode                       | Recovery                                                                                                                             |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| Activity worker dies mid-execution | `last_heartbeat` ages out (per-activity `heartbeat_timeout_secs`); engine re-queues per retry policy.                                |
| Workflow worker dies mid-replay    | `dispatch_last_heartbeat` ages out (`ASSAY_WF_DISPATCH_TIMEOUT_SECS`, default 30s); any worker on the queue picks it up and replays. |
| Engine dies                        | All state in the DB. On restart, in-flight tasks become claimable again as heartbeats age out.                                       |

`ctx:side_effect` is the escape hatch for any operation that would produce different values across
replays (current time, random IDs, external HTTP). The result is recorded once on first execution
and returned from cache thereafter, even after a worker crash.

### Schedules (cron)

Declarative recurring workflow starts. Scheduler runs on the leader node under Postgres; fires once
across the cluster.

```sh
assay schedule create nightly \
  --type Report \
  --cron "0 0 2 * * *" \
  --timezone Europe/Berlin \
  --input '{"lookback_hours":24}'

assay schedule patch   nightly --cron "0 0 3 * * *"   # in-place update (v0.11.3)
assay schedule pause   nightly                        # scheduler skips paused (v0.11.3)
assay schedule resume  nightly                        # recomputes next_run_at from now
assay schedule delete  nightly
```

Cron uses the 6- or 7-field form (with seconds). `"0 * * * * *"` = every minute on the zero second.

**Timezone (v0.11.3).** IANA name via `--timezone`. Default is `UTC`. The scheduler evaluates the
cron in that zone; `next_run_at` is persisted as a UTC epoch.

### Search attributes

Indexed application-level metadata for filtering workflows. Set at `start`, updated at runtime via
`ctx:upsert_search_attributes`, filtered on `list`:

```lua
-- set at start
workflow.start({
  workflow_type = "Ingest",
  workflow_id   = "ing-42",
  search_attributes = { env = "prod", tenant = "acme" },
})

-- update inside a running workflow
ctx:upsert_search_attributes({ progress = 0.5, stage = "deploy" })

-- filter list results (URL-encoded JSON server-side)
workflow.list({ search_attrs = { env = "prod" } })
```

Postgres backs search with a `JSONB` column + `->>` operator; SQLite uses `json_extract`. Filters
AND-join; unchanged keys are preserved across upserts.

### Dashboard

`/workflow/` (or just `/` — redirects). Real-time monitoring + tier-1 operator controls.

| View      | Read                                                | Mutate                                                                            |
| --------- | --------------------------------------------------- | --------------------------------------------------------------------------------- |
| Workflows | List + filter (status, type, search_attrs)          | `+ Start workflow` form; per-row Signal / Cancel / Terminate                      |
| Detail    | Metadata, event timeline, children, live state      | Signal / Cancel / Terminate / Continue-as-new; live `ctx:register_query` snapshot |
| Schedules | List with timezone + paused state                   | Create (with timezone) / Edit (PATCH) / Pause / Resume / Delete                   |
| Workers   | Identity, queue, last heartbeat                     | —                                                                                 |
| Queues    | Pending + running per queue                         | —                                                                                 |
| Settings  | Engine version, build profile, namespaces, API docs | Namespace create / delete                                                         |

Status-bar footer always shows the engine version (fetched from `/api/v1/version`). Live list
updates via SSE. Cache-busted asset URLs per startup.

### Concepts

| Concept           | Meaning                                                                                                                               |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| Activity          | A unit of concrete work with at-least-once semantics. Result is persisted before the workflow proceeds. Configurable retry + timeout. |
| Workflow          | Deterministic orchestration of activities, sleeps, signals, child workflows. Full event history persisted; crashed worker → replay.   |
| Task queue        | Named queue workers subscribe to. Workflows are routed to a queue; only workers on that queue claim them.                             |
| Namespace         | Logical tenant. Workflows / schedules / workers are namespace-scoped. Default `main`.                                                 |
| Signal            | Async message to a running workflow; consumed via `ctx:wait_for_signal`.                                                              |
| Schedule          | Cron expression that starts a workflow recurringly. Leader-elected under Postgres so only one instance fires.                         |
| Child workflow    | Workflow started by another workflow. Cancellation propagates parent → child recursively.                                             |
| Side effect       | Non-deterministic op captured in history on first call so replays see the same value.                                                 |
| Query handler     | `ctx:register_query` surface exposing live workflow state via `/state[/{name}]`. Snapshot written on every replay.                    |
| Search attributes | Indexed metadata (JSON object) for filtering workflows; updatable at runtime.                                                         |
| Archival stub     | Terminal workflow moved to S3 by the optional archiver; row stays in Postgres with `archive_uri` pointing at the bundle.              |

### Example — approval-gated deploy with live state

```lua
local workflow = require("assay.workflow")
workflow.connect("http://assay.example.com", { token = env.get("ASSAY_TOKEN") })

workflow.define("ApproveAndDeploy", function(ctx, input)
  local state = { stage = "build", progress = 0 }
  ctx:register_query("pipeline_state", function() return state end)

  local artifact = ctx:execute_activity("build", { ref = input.git_sha })
  state.stage = "awaiting_approval"; state.progress = 0.33

  local approval = ctx:wait_for_signal("approve")
  state.stage = "deploying"; state.progress = 0.66
  ctx:upsert_search_attributes({ approver = approval.by })

  local result = ctx:execute_activity("deploy", {
    image = artifact.image, env = input.target_env, approver = approval.by,
  })
  state.stage = "done"; state.progress = 1.0
  return result
end)

workflow.activity("build",  function(ctx, input) --[[ ... ]] end)
workflow.activity("deploy", function(ctx, input) --[[ ... ]] end)

workflow.listen({ queue = "deploys" })  -- blocks
```

Drive it from the shell:

```sh
assay workflow start --type ApproveAndDeploy --id deploy-1234 \
  --input '{"git_sha":"abc123","target_env":"staging"}'

assay workflow state deploy-1234 pipeline_state   # "awaiting_approval"

assay workflow signal deploy-1234 approve '{"by":"alice"}'

assay workflow wait deploy-1234 --timeout 300   # exit 0 on COMPLETED, 1 on failure, 2 on timeout
```

### Notes

- The whole engine + dashboard + Lua client is gated behind the `workflow` cargo feature (default
  on). To build assay without it:
  `cargo install assay-lua --no-default-features --features cli,db,server`.
- The cron crate requires **6- or 7-field** expressions. The common 5-field form fails to parse.
- The engine is also publishable as a standalone Rust crate (`assay-workflow`) for embedding in
  non-Lua Rust apps. The CLI injects its own `CARGO_PKG_VERSION` via
  `assay_workflow::api::serve_with_version` so `/api/v1/version` reflects the user-facing binary
  version, not the internal crate version.
- S3 archival is behind the `s3-archival` cargo feature (default off) and no-op at runtime unless
  `ASSAY_ARCHIVE_S3_BUCKET` is set.
