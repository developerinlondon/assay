# Changelog

All notable changes to Assay are documented here.

## [0.11.9] - 2026-04-17

### Added

- **`ctx:wait_for_signal(name, { timeout = seconds })` — bounded signal wait.**
  Returns the signal's JSON payload when a matching signal arrives within the
  timeout, or `nil` when the timer expires first. Enables approval gates,
  external-callback waits, and any workflow that must abandon its wait after a
  deadline — without a side-channel timer or manual race logic in user code.

  The call yields a batch of two commands (`ScheduleTimer` + `WaitForSignal`);
  on replay the winner is decided by comparing history event seqs of the next
  unconsumed `SignalReceived` against the paired `TimerFired`. Determinism
  matches `ctx:sleep` and `ctx:execute_parallel`.

  Backward compatible: `ctx:wait_for_signal(name)` without opts is unchanged.

### Changed

- `WaitForSignal` engine command accepts an optional `timer_seq`. When present,
  it is recorded in the `WorkflowAwaitingSignal` event payload so the dashboard
  can show which timer is racing the wait.

### Tests

- Two new orchestration tests (`orchestration.rs`):
  - `lua_workflow_wait_for_signal_timeout_signal_wins` — signal arrives before
    the 30s timer; workflow completes with the payload.
  - `lua_workflow_wait_for_signal_timeout_timer_wins` — no signal sent; the 1s
    timer fires and the workflow completes with the timeout branch.

## [0.11.8] - 2026-04-17

### Changed

- **`GET /api/v1/health` and `GET /api/v1/version` are now always
  unauthenticated,** regardless of whether `--auth-issuer` or
  `--auth-api-key` is set. Standard practice for liveness/readiness
  probes and version discovery — Kubernetes kubelet, load balancers,
  third-party monitors, and the CLI can now reach these endpoints
  without a bearer token.

  Previously both endpoints lived inside the auth-gated `/api/v1/*`
  surface, which forced `workflow.connect()`'s connectivity probe and
  kubelet probes to carry a valid credential. That blocked legitimate
  first-boot bootstrap flows (e.g. the gitops reconcile script trying
  to `POST /api/v1/api-keys` through the unauth bootstrap window had
  to sidestep `workflow.connect` entirely).

  All other `/api/v1/*` endpoints remain authenticated when auth is
  enabled.

### Internal

- `api/public.rs` — new module that owns the public (unauth) sub-router
  at `/api/v1/*`. Holds `health_check` + `version`.
- `api/meta.rs` deleted — its single `/version` route moved to
  `api/public.rs`. The `VersionInfo` struct moved with it.
- `api/workers.rs` no longer registers `/health`. Its single responsibility
  is `/workers` now.
- `api/mod.rs` `router()` grew a third tier alongside
  "authenticated /api/v1/*" and "dashboard + openapi": "public /api/v1/*",
  merged outside the auth middleware layer by construction.

### Tests

- Five new auth tests (`auth_test.rs`) verify `/api/v1/health` returns
  200 unauth in api-key / jwt / combined modes, that `/api/v1/version`
  is unauth in api-key mode, and that other `/api/v1/*` paths still
  require auth (regression guard against accidentally opening up more
  of the surface).

## [0.11.7] - 2026-04-17

### Added

- **`POST /api/v1/api-keys` endpoint** — REST alternative to the
  `assay serve --generate-api-key` CLI subcommand. Accepts
  `{ label?, idempotent? }`. With `idempotent=true` and a key matching
  the label already exists, returns `200 OK` with the existing record's
  metadata (no plaintext). Otherwise mints a fresh key and returns
  `201 Created` with the plaintext.

  **Bootstrap window:** when the `api_keys` table is empty, `POST
  /api/v1/api-keys` is callable without authentication. This is the
  only way a freshly deployed server running in API-key or combined
  mode can receive its first credential without operator shell access.
  The window closes the moment any key exists.

- **`GET /api/v1/api-keys`** and **`DELETE /api/v1/api-keys/{prefix}`** —
  list and revoke.

- **`workflow.api_keys.{generate, list, delete}`** Lua stdlib helpers
  wrapping the above endpoints. Example:

  ```lua
  local resp = workflow.api_keys.generate("cc_api_key", { idempotent = true })
  if resp.plaintext then
      -- fresh mint; persist plaintext somewhere (e.g. a k8s Secret)
  else
      -- already exists; plaintext was issued on first call
  end
  ```

### Store

- New `WorkflowStore` trait methods: `api_keys_empty()` (used by the
  bootstrap-window gate) and `get_api_key_by_label(label)` (used by the
  idempotent-mode lookup). Implemented for both SQLite and Postgres.

- `ApiKeyRecord` now derives `utoipa::ToSchema` so the OpenAPI spec
  includes it.

### Changed

- **`assay-workflow` crate** bumped to `0.1.5` (from `0.1.4`). Additive
  API changes; downstream consumers on `version = "0.1"` continue to work.

## [0.11.6] - 2026-04-17

### Fixed

- **Postgres schema migration crash on startup.** `PostgresStore::migrate()` split
  the embedded `SCHEMA` string by `;` and executed each fragment as SQL. A semicolon
  inside an SQL line comment (`-- Idempotent across startups; fresh installs pick
  the column up from the…`) produced a phantom fragment starting with naked prose,
  and Postgres rejected it with `syntax error at or near "fresh"` — which crashed
  `assay serve` on every boot against a Postgres backend, regardless of whether the
  target database was fresh or already populated. Affects v0.11.3 through v0.11.5.

  Fix: extract the split into a `sanitise_schema` helper that drops pure-comment
  lines (leading whitespace then `--`) before splitting on `;`. Inline `--`-after-code
  and string-literal contents are left untouched, so the filter is conservative enough
  to stay correct as the SCHEMA grows more prose.

### Changed

- **`assay-workflow` crate** bumped to `0.1.4` (from `0.1.3`). No public API
  changes. Downstream consumers on `version = "0.1"` continue to work.

### Tests

- Added five pure-Rust unit tests for `sanitise_schema` under `src/store/postgres.rs`
  that run on all platforms — no Docker required. Includes a regression test
  (`sanitise_schema_real_constant_produces_only_ddl`) that asserts the live `SCHEMA`
  constant never produces a statement whose first token isn't a recognised SQL
  keyword. This would have caught the v0.11.3 bug at CI time; the existing
  integration tests under `tests/postgres_store.rs` skip when Docker is unavailable
  (macOS default), which is why this class of bug slipped through.

## [0.11.5] - 2026-04-17

### Changed

- **`assay-workflow` crate version** bumped to `0.1.3` (from `0.1.2`) — carries the
  v0.11.4 `AuthMode` refactor from enum to struct. Per assay's pre-1.0 policy of
  patch-bumps-by-default, both crates stay in their current minor tracks until there's
  a deliberate decision to signal API instability to downstream consumers.

### Fixed

- **crates.io publish.** v0.11.4 shipped the binary (GHCR, npm, Linux/macOS artefacts,
  GitHub release) but its crates.io publish failed because `assay-workflow` was still
  pinned to `0.1.2` — the same version already published for v0.11.3. v0.11.5 is a
  re-release of v0.11.4's code with both crates' versions bumped so the publish
  actually lands on crates.io.

### Docs

- `AGENTS.md` "Release docs checklist" gains an explicit note about `crates/*/Cargo.toml`
  and the independent-versioning policy for sub-crates — the gap that caused the v0.11.4
  crates.io failure.

## [0.11.4] - 2026-04-17

### Added

- **Combined JWT + API-key authentication for `assay serve`.** `--auth-issuer` and
  `--auth-api-key` can now be set on the same invocation. When both are enabled, the
  auth middleware dispatches on token shape:

  - Bearer tokens that parse as a JWS header are validated against the OIDC issuer's
    JWKS.
  - Bearer tokens that are not JWT-shaped are hashed and looked up in the API-key
    store.

  A semantically-invalid JWT (expired, wrong issuer / audience, forged signature) is
  rejected on the JWT path and is **not** retried as an API key — a token that looks
  like a JWT is treated as a JWT. This lets a single server accept short-lived OIDC
  user tokens from a browser session and long-lived machine API keys from a CI job
  without the caller picking a mode up front.

### Changed

- **`AuthMode` is now a struct** (`api_key: bool`, `jwt: Option<JwtConfig>`) instead of
  an enum with three variants. Library constructors are unchanged in shape —
  `AuthMode::no_auth()`, `AuthMode::api_key()`, `AuthMode::jwt(issuer, audience)` — and
  a new `AuthMode::combined(issuer, audience)` enables both paths. `AuthMode::is_enabled()`
  replaces `!matches!(.., NoAuth)` call sites.

  Breaking for downstream Rust consumers that matched on `AuthMode::NoAuth | ApiKey |
  Jwt { .. }`. The `assay` binary and REST / dashboard users are unaffected.

### Docs

- `docs/modules/workflow.md` auth table adds the combined-mode row and documents the
  token-shape dispatch rule.

## [0.11.3] - 2026-04-16

### Added

- **`ctx:register_query`** — Lua workflows can expose live application-level state to external
  callers via named query handlers. After every worker replay the engine persists a snapshot of
  every handler's result; two new REST endpoints surface it:

  ```
  GET /api/v1/workflows/{id}/state         → latest full snapshot
  GET /api/v1/workflows/{id}/state/{name}  → one handler's value
  ```

  Workflows that don't call `register_query` pay nothing — the worker skips the snapshot command
  entirely when no handlers are registered. A handler that raises is dropped from the snapshot
  rather than crashing the workflow (queries are best-effort read-through).

- **`ctx:continue_as_new`** — Lua surface for the engine-level `continue_as_new` REST endpoint that
  already existed. Workflows yield a `ContinueAsNew` command and the engine closes out the current
  run, starts a fresh one with the same type / namespace / task_queue under `{id}-continued-{ts}`
  with the caller-supplied input and empty event history. Standard pattern for unbounded-loop
  workflows (pollers, schedulers) whose event log would otherwise grow forever.

- **`ctx:execute_parallel`** — Run multiple activities concurrently from a single handler run. The
  worker yields a batch of `ScheduleActivity` commands; the engine schedules them idempotently on
  `(workflow_id, seq)`. Each completion re-dispatches the workflow, replay cache-hits for completed
  activities and re-yields the rest (no-op at the store layer). The handler proceeds past the call
  only when every activity has a terminal event. Per-activity retry / timeout opts match
  `ctx:execute_activity`.

  ```lua
  local results = ctx:execute_parallel({
      { name = "check_a", input = { id = 1 } },
      { name = "check_b", input = { id = 2 }, opts = { max_attempts = 5 } },
      { name = "check_c", input = { id = 3 } },
  })
  -- results[1], [2], [3] in input order; raises if any fail after retries.
  ```

- **`ctx:upsert_search_attributes`** + **search attributes on workflows** — Workflows gain a
  `search_attributes` JSON object settable at start (`POST /workflows` body) and updatable at
  runtime (`ctx:upsert_search_attributes({ … })`). The list endpoint accepts a URL-encoded JSON
  filter that matches workflows whose attributes contain every listed key at the given value:

  ```
  GET /api/v1/workflows?search_attrs=%7B%22env%22%3A%22prod%22%7D
  ```

  SQLite uses `json_extract`; Postgres uses `(search_attributes::jsonb)->>'key'`. Filters AND-join.
  Unchanged keys are preserved across upserts.

- **Schedule `PATCH` / `pause` / `resume`** — Schedules can be updated in place without a
  delete-and-recreate. Only fields present on the patch are touched; unchanged fields keep their
  existing values.

  ```
  PATCH /api/v1/schedules/{name}  body: { cron_expr?, timezone?, input?, task_queue?, overlap_policy? }
  POST  /api/v1/schedules/{name}/pause
  POST  /api/v1/schedules/{name}/resume
  ```

  Paused schedules are skipped by the scheduler; resume recomputes `next_run_at` from now and does
  not backfill missed fires. Updates take effect within a scheduler tick (≤15s).

- **Cron timezone** — Schedules gain a `timezone` field (IANA name, e.g. `"Europe/Berlin"`,
  `"America/New_York"`). Default `"UTC"` preserves v0.11.2 behaviour. The scheduler parses the
  timezone via `chrono-tz` and evaluates the cron expression in that zone, then persists the UTC
  epoch as `next_run_at`. Invalid names are rejected at create / patch time.

- **Optional S3 archival for completed workflows** — Behind the `s3-archival` cargo feature
  (default-off). When compiled in and `ASSAY_ARCHIVE_S3_BUCKET` is set at runtime, a background task
  periodically finds workflows in terminal states older than `ASSAY_ARCHIVE_RETENTION_DAYS` (default
  30), bundles `{workflow_record, events}` as JSON, uploads to
  `s3://bucket/prefix/<namespace>/<workflow_id>.json`, and purges dependent rows (events,
  activities, timers, signals, snapshots). The workflow row itself is retained with `archived_at` +
  `archive_uri` set so `GET /workflows/{id}` still resolves with a pointer to the cold-storage
  bundle.

  Credentials resolve via the AWS SDK's default chain — env vars, shared config, or IRSA /
  pod-identity via web-identity token. Other env vars: `ASSAY_ARCHIVE_S3_PREFIX` (default `assay/`),
  `ASSAY_ARCHIVE_POLL_SECS` (default 3600), `ASSAY_ARCHIVE_BATCH_SIZE` (default 50).

- **`assay.workflow` Lua stdlib — full management surface.** The stdlib now covers every REST
  endpoint the engine exposes, so Lua scripts (including CC and Kubernetes Jobs running
  `assay run seed.lua`) can manage workflows, schedules, namespaces, workers, and queues without
  hand-rolling HTTP calls. New top-level functions:

  ```
  workflow.list(opts)              workflow.list_children(id)
  workflow.terminate(id, reason)   workflow.continue_as_new(id, input)
  workflow.get_events(id)          workflow.get_state(id, name?)
  ```

  New sub-tables (each exposes `create / list / describe / patch / pause / resume / delete` as
  applicable):

  ```
  workflow.schedules   workflow.namespaces   workflow.workers   workflow.queues
  ```

  Every function is a thin HTTP wrapper returning the parsed JSON response (or `nil` on a 404 for
  `describe`/`get_state`), raising on other non-2xx responses.

- **Full CLI for the workflow engine.** The clap-registered `assay workflow …` / `assay schedule …`
  subcommands that through v0.11.2 printed "not yet implemented" and exited 1 are replaced with real
  REST-client implementations, plus a considerable expansion. Everything visible in `assay --help`
  actually runs.

  **Subcommand trees:**

  ```
  assay workflow
    start --type T [--id ID] [--input JSON] [--queue Q] [--search-attrs JSON]
    list [--status S] [--type T] [--search-attrs JSON] [--limit N]
    describe <id>
    state <id> [<query-name>]                   # register_query reader
    events <id> [--follow]                      # log, or poll-stream until terminal
    children <id>
    signal <id> <name> [payload-as-json-or-@file-or--]
    cancel <id>
    terminate <id> [--reason R]
    continue-as-new <id> [--input JSON]         # client-side
    wait <id> [--timeout SECS] [--target STATUS]  # exit 0/1/2 for scripts

  assay schedule
    list
    describe <name>
    create <name> --type T --cron EXPR [--timezone TZ] [--input JSON] [--queue Q]
    patch <name> [--cron EXPR] [--timezone TZ] [--input JSON] [--queue Q] [--overlap POLICY]
    pause <name>
    resume <name>
    delete <name>

  assay namespace   create | list | describe | delete
  assay worker      list
  assay queue       stats
  assay completion  <bash|zsh|fish|powershell|elvish>
  ```

  **Global options** (all flag-backed, env-backed, and config-file-backed, resolved in that
  precedence order):

  - `--engine-url` / `ASSAY_ENGINE_URL` (default `http://127.0.0.1:8080`)
  - `--api-key` / `ASSAY_API_KEY` (bearer token, forwarded as `Authorization: Bearer <value>`)
  - `--namespace` / `ASSAY_NAMESPACE` (default `main`)
  - `--output` / `ASSAY_OUTPUT` — `table` | `json` | `jsonl` | `yaml`; TTY-adaptive default (`table`
    on a terminal, `json` when stdout is piped)
  - `--config` / `ASSAY_CONFIG_FILE` — YAML config file, discovered in this order: flag → env →
    `$XDG_CONFIG_HOME/assay/config.yaml` → `~/.config/assay/config.yaml` → `/etc/assay/config.yaml`

  **Config file** (every field optional):

  ```yaml
  engine_url: https://assay.example.com
  api_key_file: /run/secrets/assay-api-key # preferred over `api_key:`
  namespace: main
  output: table
  ```

  `api_key_file` reads the file contents, trims whitespace, and uses that as the bearer token. Lets
  the config live in a ConfigMap with the credential in a separate Secret.

  **JSON input indirection.** `--input`, `--search-attrs`, and signal payload args accept:

  - a literal JSON string (`'{"n":1}'`)
  - `@PATH` — read the file and parse
  - `-` — read stdin and parse

  **Exit codes:** 0 success, 1 HTTP error / unreachable / not-found, 2 `workflow wait` timeout, 64
  usage error (bad JSON input).

  **Shell completion.** `assay completion <shell> > /etc/bash_completion.d/assay` (or the equivalent
  for your shell). Buffered and graceful on SIGPIPE so piping to `head` doesn't panic. Adds one new
  crate dep: `clap_complete`.

- **Tier-1 dashboard mutations.** The built-in dashboard at `/workflow/` was read-only through
  v0.11.2; every existing view now pairs with its matching operator control:

  - **Workflows view** — new `+ Start workflow` inline form (type / id / task_queue / input JSON /
    search_attributes JSON); per-row Signal / Cancel / Terminate; search-attributes filter in the
    toolbar (debounced, with client-side JSON validation).
  - **Workflow detail panel** — Signal, Cancel, Terminate, and Continue-as-new buttons, all with
    toast feedback. "Live state" card renders the latest snapshot written by `ctx:register_query`
    handlers (with the event seq and timestamp the snapshot was taken at).
  - **Schedules view** — per-row Edit (PATCH form pre-filled with the schedule's values), Pause /
    Resume toggle, Delete. Create form picks up a Timezone field.
  - **Settings view** — Engine Info card shows the engine version + build profile, fetched from
    `/api/v1/version`. Namespace create / delete upgraded to toast feedback and refreshes the
    sidebar namespace switcher.
  - Shared `toast()` + `apiFetchRaw()` helpers exposed via the component context for consistent
    success/error feedback across every mutation.

  Explicitly tier 1 — no in-browser workflow authoring, no batch operations, no reset-to-event, no
  in-browser RBAC. Those are tier 2 / tier 3 and deferred to later releases.

- **`GET /api/v1/version` endpoint.** Returns `{ version, build_profile }`. The CLI passes its own
  `CARGO_PKG_VERSION` to `assay_workflow::api::serve_with_version`, so the field reflects the
  user-facing binary (e.g. `0.11.3`) and not the internal `assay-workflow` crate version. Embedders
  using plain `serve` get the crate version as a fallback. `AppState` gains a
  `binary_version: Option<&'static str>` field.

### Changed

- **`Engine::start_workflow` signature** gains a `search_attributes: Option<&str>` parameter (for
  embedders using the crate directly). REST callers are unaffected; the field is optional on
  `StartWorkflowRequest`.

- **`WorkflowStore::list_workflows` signature** gains a `search_attrs_filter: Option<&str>`
  parameter (for embedders).

- **`WorkflowSchedule`** struct gains a `timezone: String` field. Deserialisers that accept the type
  from an older v0.11.2 engine will need to tolerate the missing field (default "UTC").

- **`WorkflowRecord`** struct gains `search_attributes`, `archived_at`, `archive_uri` fields.

### Fixed

- Removed three pre-existing `clippy::map_identity` warnings in orchestration test helpers so
  `cargo clippy --tests -- -D warnings` stays clean under rust 1.92 / clippy 1.91.

### Notes

- **No migrations from v0.11.2.** The engine is pre-1.0 and no v0.11.x release has been deployed
  against a real workload yet, so all v0.11.3 columns (`search_attributes`, `archived_at`,
  `archive_uri` on `workflows`; `timezone` on `workflow_schedules`) live in the baseline
  `CREATE TABLE` statements only. A fresh DB picks them up automatically; an existing v0.11.2 DB
  needs to be recreated. The migration plumbing is kept in place for post-v0.11.3 additive
  migrations — Postgres does `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` natively, SQLite has a
  dormant `add_column_if_missing` helper that pragma-checks before ALTER. The pattern is documented
  at the bottom of each store's `SCHEMA` constant / `migrate()` fn.
- Parallel activities are still best-effort in the sense that each completion triggers a replay;
  deeply parallel fan-outs generate O(N²) idempotent `schedule_activity` calls. The store-level
  idempotency makes this correct but not minimal; a follow-up can short-circuit re-yields for
  already-scheduled seqs.

## [0.11.2] - 2026-04-16

### Fixed

- **Docker image build** — `Dockerfile` now `COPY crates/` so the `assay-workflow` workspace
  member's manifest is in the build context. Without this, the v0.11.1 release.yml docker job failed
  with `failed to read /app/crates/assay-workflow/Cargo.toml` and no
  `ghcr.io/developerinlondon/assay:v0.11.1` image was published. v0.11.2 republishes everything
  (binaries / crates.io / npm / docker) so `:latest` points at a working image again.

### Notes

- No source-level changes versus v0.11.1 — `assay-lua` and `assay-workflow` crates are
  byte-identical to v0.11.1 except for the version bumps. Existing v0.11.1 binaries, crates.io
  packages, and npm packages remain valid; only the GHCR image was missing.

## [0.11.1] - 2026-04-16

### Added

- **`assay serve`** — Native durable workflow engine built into assay. One binary, multiple modes:
  `assay serve` runs the engine; `assay run worker.lua` runs a worker; `assay workflow` /
  `assay schedule` manage from the shell. Replaces the need for external workflow infrastructure
  (Temporal, Celery, Inngest).

- **Deterministic-replay runtime** — Workflow code is plain Lua run as a coroutine; each `ctx:` call
  gets a per-execution sequence number and the engine persists every completed command
  (`ActivityCompleted`, `TimerFired`, `SignalReceived`, `SideEffectRecorded`,
  `ChildWorkflowCompleted`, …). On replay, `ctx:` calls short-circuit to cached values for
  everything in history; only the next unfulfilled step actually runs. This is how worker crashes
  don't lose work and side effects don't duplicate.

- **Crash safety** — Three independent recovery layers:
  - Activity worker dies → `last_heartbeat` ages out per-activity; engine re-queues per retry
    policy.
  - Workflow worker dies → `dispatch_last_heartbeat` ages out (`ASSAY_WF_DISPATCH_TIMEOUT_SECS`,
    default 30s); any worker on the queue picks up and replays from the event log.
  - Engine dies → all state is in the DB; in-flight tasks become claimable again as heartbeats age
    out. Verified by an end-to-end SIGKILL test in the orchestration suite.

- **Workflow handler context (`ctx`)** — `ctx:execute_activity` (sync, returns result, raises on
  failure after retries), `ctx:sleep(seconds)` (durable timer; survives worker bouncing),
  `ctx:wait_for_signal(name)` (block until matching signal arrives, returns its payload),
  `ctx:start_child_workflow(type, opts)` (sync, parent waits for child), `ctx:side_effect(name, fn)`
  (run non-deterministic op exactly once, cache in event log).

- **REST API** (~25 endpoints) — Workflow lifecycle (`start`, `list`, `describe`, `signal`,
  `cancel`, `terminate`, `continue-as-new`, `events`, `children`); workflow-task dispatch
  (`/workflow-tasks/poll`, `/workflow-tasks/:id/commands`); activity scheduling
  (`/workflows/:id/activities`, `/activities/:id`); worker registration & polling; schedule CRUD;
  namespace CRUD; queue stats. All documented in the served OpenAPI spec.

- **OpenAPI spec** — Machine-readable spec at `/api/v1/openapi.json`. Interactive docs at
  `/api/v1/docs` (Scalar). Enables auto-generation of typed client SDKs in any language via
  `openapi-generator`.

- **Built-in dashboard** — Real-time workflow monitoring at `/workflow/`, brand-aligned with
  [assay.rs](https://assay.rs). Light/dark theme, foldable sidebar, favicon. Six views: Workflows
  (list with status filter, drill-in to event timeline + children), Schedules (list + create),
  Workers (live status + active task count), Queues (pending/running stats
  - warnings when no worker is registered), Namespaces, Settings. Live updates via SSE. Cache-busted
    asset URLs (per-process startup stamp) so a deploy is reflected immediately.

- **Provider-agnostic auth** — Three modes: no-auth (default), API keys (SHA256-hashed in DB),
  JWT/OIDC (validates against any OIDC provider via JWKS with caching, e.g. Cloudflare Access,
  Auth0, Okta, Dex, Keycloak). CLI: `--generate-api-key`, `--list-api-keys`, `--auth-issuer`,
  `--auth-audience`, `--auth-api-key`.

- **Multi-namespace** — Logical-tenant isolation. Workflows / schedules / workers in one namespace
  are invisible to others. Default `main`. CRUD via REST + dashboard.

- **Postgres + multi-instance** — Same engine, swap the backend with `--backend postgres://...` or
  `DATABASE_URL=...`. Cron scheduler uses `pg_try_advisory_lock` for leader election so only one
  instance fires schedules. Activity
  - workflow-task claiming uses `FOR UPDATE SKIP LOCKED` so multiple engine instances don't race.
    SQLite is single-instance only (engine takes an `engine_lock` row at startup).

- **`assay.workflow` Lua stdlib module** — `workflow.connect()`, `workflow.define()`,
  `workflow.activity()`, `workflow.listen()`, plus `workflow.start()` / `signal()` / `describe()` /
  `cancel()` for client-side control. The same `listen()` loop drives both workflow handlers and
  activity handlers — one process, both roles.

- **`examples/workflows/`** — Three runnable examples with READMEs: `hello-workflow/` (smallest
  case), `approval-pipeline/` (signal-based pause/resume), `nightly-report/` (cron + side_effect +
  child workflows).

- **`assay-workflow` crate** — The workflow engine is also publishable as a standalone Rust crate
  (`assay-workflow = "0.1"`) for embedding in non-Lua Rust applications. Zero Lua dependency.

- **SSE client in `http.get`** — Auto-detects `text/event-stream` responses and streams events to an
  `on_event` callback. Backwards compatible with existing `http.get` usage.

### Tests

- **17 end-to-end orchestration tests** (`crates/assay-workflow/tests/orchestration.rs`) including 9
  that boot a real assay subprocess and verify a full workflow runs to a real result. Highlights:
  - `lua_workflow_runs_to_completion` — two sequential activities, real result.
  - `lua_workflow_with_durable_timer` — `ctx:sleep(1)` actually pauses ~1s and resumes.
  - `lua_workflow_with_signal` — workflow blocks, test sends signal, workflow completes with the
    payload bubbled into the result.
  - `lua_workflow_cancellation_stops_work` — cancel mid-sleep; activity that was about to run is
    never scheduled.
  - `lua_workflow_side_effect_is_recorded_once` — side-effect counter file shows fn ran exactly once
    across all replays.
  - `lua_child_workflow_completes_before_parent` — parent + child each run as proper workflows,
    parent picks up child's result.
  - `lua_cron_schedule_fires_real_workflow` — schedule fires within the scheduler tick, workflow
    completes, result lands in DB.
  - `lua_worker_crash_resumes_workflow` — SIGKILL worker A mid-flight; worker B takes over via
    heartbeat-timeout release; workflow completes; side-effect counter still shows exactly one
    execution.

- **11 REST-level tests** (no Lua subprocess) covering scheduling, completion, retries,
  workflow-task dispatch, command processing.

- **10 Postgres tests** (testcontainers-backed) verifying store CRUD parity against a real Postgres
  instance.

### Notes

- The cron crate (`cron = "0.16"`) requires 6- or 7-field cron expressions (with seconds). The
  5-field form fails to parse — use `0 * * * * *` for "every minute on the zero second" or
  `* * * * * *` for "every second."
- The whole engine is gated behind the `workflow` cargo feature (default-on). To build assay without
  it: `cargo install assay-lua --no-default-features --features cli,db,server`.
- Parallel activities (Promise.all-style) are not yet supported; tracked as a follow-up. Sequential
  `ctx:execute_activity` calls and child workflows cover most patterns today.

## [0.11.0] - 2026-04-15

### Removed

- **Temporal integration** — The `temporal` feature flag and all Temporal SDK dependencies
  (`temporalio-client`, `temporalio-sdk`, `temporalio-sdk-core`, `temporalio-common`,
  `prost-wkt-types`) have been removed. The gRPC client (`temporal.connect()`, `temporal.start()`),
  worker runtime (`temporal.worker()`), and HTTP REST stdlib module (`require("assay.temporal")`)
  are no longer available. The Temporal integration never reached production stability and required
  an external Temporal cluster plus `protoc` at build time. A native workflow engine (`assay serve`)
  is planned for v0.11.1.

### Changed

- **Binary size** — 16MB → 11MB (-5MB) with Temporal dependencies removed.
- **Build time** — ~90s → ~34s. `protoc` is no longer required at build time.
- **Stdlib module count** — 35 → 34 (temporal module removed).

## [0.10.4] - 2026-04-12

### Added

- **`os.date(format?, time?)`** — Standard Lua time formatting. Supports strftime patterns (`%Y`,
  `%m`, `%d`, `%H`, `%M`, `%S`, `%c`), the `!` prefix for UTC, and `*t` table output. Previously
  missing from the sandboxed environment.
- **`os.time()`** — Returns current UTC epoch as integer (standard Lua).
- **`os.clock()`** — Returns CPU time in seconds (standard Lua).

## [0.10.3] - 2026-04-12

### Added

- **`ctx:register_query(name, handler)`** — Register query handlers in Temporal workflows. The
  handler function is called when Temporal dispatches a QueryWorkflow activation, and the result is
  returned as a JSON payload. Enables dashboard-style apps to read workflow state in real-time
  without signals.

- **`kratos.flows:get_login_admin(flow_id)`** — Fetch a login flow via the Kratos admin API (no CSRF
  cookie required). Server-side components like hydra-auth should use this instead of `get_login()`
  which requires browser cookies that may not be available across different cookie domains.

## [0.10.1] - 2026-04-12

### Fixed

- **Temporal worker identity** — `temporal.worker()` and `temporal.connect()` now set a non-empty
  `identity` on `ConnectionOptions`. The Temporal SDK v0.2.0 requires this field; without it,
  `init_worker` fails with "Client identity cannot be empty". Identity is set to
  `assay-worker@{task_queue}` for workers and `assay-client@{namespace}` for clients.

## [0.10.0] - 2026-04-11

### Added

- **`assay.gitlab`** — GitLab REST API v4 client. Full coverage of projects, repository files,
  atomic multi-file commits, branches, tags, merge requests, pipelines, jobs, releases, issues,
  groups, container registry, webhooks, environments, deploy tokens, and user endpoints. Supports
  both private access token and OAuth2 bearer authentication. Enables GitOps automation scripts to
  read/write repository content, trigger pipelines, manage merge requests, and interact with
  container registries without external CLI dependencies.

### Changed

- **Sub-object OO convention** across all 35 stdlib modules. Methods are now grouped by resource
  into sub-objects instead of flat on the client:

  ```lua
  -- Before (flat)
  c:merge_requests(project, opts)
  c:create_merge_request(project, opts)

  -- After (sub-objects)
  c.merge_requests:list(project, opts)
  c.merge_requests:create(project, opts)
  ```

  Standard CRUD verbs (`list`, `get`, `create`, `update`, `delete`) are consistent across all
  resources. This makes the API more intuitive and self-documenting. Modules refactored: gitlab,
  github, argocd, vault, s3, unleash, grafana, keto, kratos, hydra, rbac, prometheus, alertmanager,
  traefik, loki, k8s, harbor, temporal, dex, flux, certmanager, eso, crossplane, velero, kargo,
  gcal, gmail, openclaw, zitadel, postgres. Modules unchanged (no client pattern): healthcheck,
  oauth2, email_triage, openbao (alias).

## [0.9.0] - 2026-04-11

### Added

- **Temporal workflow engine** — full workflow execution via Lua coroutines. `temporal.worker()` now
  supports both activities and workflows. Each workflow runs as a coroutine with a deterministic
  `ctx` object:

  - `ctx:execute_activity(name, input, opts?)` — schedule activity, block until complete. Supports
    retry policies, timeouts, heartbeats. On replay, returns cached results without re-executing.
  - `ctx:wait_signal(name, opts?)` — block until external signal or timeout. Signals are buffered
    (safe to call after signal arrives).
  - `ctx:sleep(seconds)` — deterministic timer via Temporal, not wall clock.
  - `ctx:side_effect(fn)` — run non-deterministic function (IDs, timestamps).
  - `ctx:workflow_info()` — workflow metadata (id, type, namespace, attempt).

  Activities and workflows can be registered together in one worker:
  ```lua
  temporal.worker({
    url = "temporal-frontend:7233",
    task_queue = "promotions",
    activities = { update_gitops = function(input) ... end },
    workflows = {
      PromotionWorkflow = function(ctx, input)
        local approval = ctx:wait_signal("approve", { timeout = 86400 })
        local commit = ctx:execute_activity("update_gitops", input)
        return { status = "done", commit_id = commit.id }
      end,
    },
  })
  ```

- **`markdown.to_html(source)`** — new builtin for Markdown to HTML conversion via pulldown-cmark.
  Supports tables, strikethrough, and task lists. Zero binary size overhead (pulldown-cmark was
  already in the dependency tree via temporalio crates).

- **`http.serve()` wildcard routes** — routes ending with `/*` match any path with that prefix. More
  specific wildcards take priority:
  ```lua
  http.serve(8080, {
    GET = {
      ["/api/*"] = function(req) ... end,  -- matches /api/users/123
      ["/*"] = function(req) ... end,      -- catches everything else
    },
  })
  ```

- **Assay builds its own documentation site**. `site/build.lua` replaces the bash/awk/npx pipeline.
  Module count (54) is computed automatically from `src/lua/builtins/mod.rs` and `stdlib/**/*.lua`.
  Site source lives under `site/`, build output goes to `build/site/` (gitignored).

- **Per-module documentation pages**. 36 markdown source files under `docs/modules/` are the single
  source of truth. `build.lua` generates individual HTML pages, a module index, and `llms-full.txt`
  for LLM agents.

- **`site/serve.lua`** — assay serves its own docs site using wildcard routes. 40 lines of Lua, zero
  external dependencies.

- **`fs.read_bytes(path)` / `fs.write_bytes(path, data)`** — binary-safe file I/O. Lua strings can
  hold arbitrary bytes, so these work for images, WASM, protobuf, compressed data, etc.

- **Pagefind search** — full-text search across all docs pages via Ctrl+K modal. Indexed at build
  time (~100KB client bundle), runs entirely in the browser.

### Changed

- **`http.serve()` binary response body** — response `body` field now preserves raw bytes (read via
  `mlua::String`) instead of forcing UTF-8 conversion. Binary assets (WASM, images) serve correctly.

- Version bump to 0.9.0 (from 0.8.4).
- Site source consolidated under `site/` (was split across `site/`, `site-partials/`,
  `site-static/`).
- Nav redesign: no underlines, subtle active page pill, frosted glass header, theme toggle
  persistence across pages.
- `deploy.yml` updated: `cargo build` → `assay site/build.lua` → wrangler deploys `build/site/`.

## [0.8.4] - 2026-04-11

### Added

- **`assay.ory.keto` — OPL permit support and table-style check()**. `k:check()` now accepts a table
  argument in addition to positional args, making OPL permit checks natural:
  ```lua
  k:check({ namespace = "command_center", object = "cc",
            relation = "trigger", subject_id = "user:uuid" })
  ```
  Keto evaluates the OPL rewrite rules and returns true/false — no Lua-side capability mapping
  needed.

- **`k:batch_check(tuples)`** — check multiple permission tuples in a single call. Returns a list of
  booleans in the same order. Each entry uses the same table format as `check()`.

- **`assay.ory.kratos` — complete self-service flow coverage**. Three flow families that were
  missing are now implemented:

  - **Registration**: `c:submit_registration_flow(flow_id, payload, cookie?)` was missing entirely,
    making the registration API unusable.
  - **Recovery** (password reset): `c:create_recovery_flow(opts?)`,
    `c:get_recovery_flow(id, cookie?)`, `c:submit_recovery_flow(flow_id, payload, cookie?)`.
  - **Settings** (profile/password change): `c:create_settings_flow(cookie)`,
    `c:get_settings_flow(id, cookie?)`, `c:submit_settings_flow(flow_id, payload, cookie?)`.

### Fixed

- **`assay.ory.keto`**: `k:delete()` now supports subject_set tuples. Previously only `subject_id`
  was passed to the query string, silently ignoring subject_set-based tuples.

- **`assay.ory.keto`**: `build_query()` now URL-encodes parameter values. Previously special
  characters in subject IDs (e.g. `@` in email addresses) were passed raw, potentially corrupting
  the query string.

- **`assay.ory.kratos`**: `public_post()` now handles HTTP 422 responses (Kratos returns 422 for
  browser flows that need a redirect after successful submission).

## [0.8.3] - 2026-04-07

### Added

- **`assay.ory.rbac`** — capability-based RBAC engine layered on top of Ory Keto. Define a policy
  once (role → capability set) and get user lookups, capability checks, and membership management
  for free. Users can hold multiple roles and the effective capability set is the union, which means
  proper separation of duties is enforceable at the authorization layer (e.g. an "approver" role can
  have `approve` without also getting `trigger`, even if it's listed above an "operator" role with
  `trigger`).

  Public surface:
  - `rbac.policy({namespace, keto, roles, default_role?})`
  - `p:user_roles(user_id)` — sorted by rank, highest first
  - `p:user_primary_role(user_id)` — for compact UI badges
  - `p:user_capabilities(user_id)` — union set
  - `p:user_has_capability(user_id, cap)` — single check
  - `p:add(user_id, role)` / `p:remove(user_id, role)` — idempotent
  - `p:list_members(role)` / `p:list_all_memberships()`
  - `p:reset_role(role)` — for bootstrap/seed scripts
  - `p:require_capability(cap, handler)` — http.serve middleware

- **`crypto.jwt_decode(token)`** — decode a JWT WITHOUT verifying its signature. Returns
  `{header, claims}` parsed from the base64url segments. Useful when the JWT travels through a
  trusted channel (your own session cookie set over TLS) and you just need to read the claims rather
  than verify them. For untrusted JWTs, verify the signature with a JWKS-aware verifier instead.

- **Nested stdlib module loading**: `require("assay.ory.kratos")` now resolves to
  `stdlib/ory/kratos.lua`. The stdlib and filesystem loaders translate dotted module paths into
  directory paths and try both `<path>.lua` and `<path>/init.lua`, matching standard Lua package
  loading conventions.

### Changed

- **BREAKING: Ory stack modules moved under `assay.ory.*`**. The flat top-level `assay.kratos`,
  `assay.hydra`, and `assay.keto` modules are now `assay.ory.kratos`, `assay.ory.hydra`, and
  `assay.ory.keto`. The convenience wrapper `require("assay.ory")` is unchanged and still returns
  `{kratos, hydra, keto, rbac}`.

  Migration: replace `require("assay.kratos")` → `require("assay.ory.kratos")`
  `require("assay.hydra")` → `require("assay.ory.hydra")` `require("assay.keto")` →
  `require("assay.ory.keto")`

  This is the right architectural shape: Ory-specific modules sit under the `assay.ory.*` umbrella
  alongside the new `assay.ory.rbac`, leaving room for `assay.<other-vendor>.*` later without
  polluting the top-level namespace.

## [0.8.2] - 2026-04-07

### Added

- **`assay.hydra` logout challenge methods**: completes the OIDC challenge trio (login, consent,
  logout). When an app calls Hydra's `/oauth2/sessions/logout` endpoint with `id_token_hint` and
  `post_logout_redirect_uri`, Hydra creates a logout request and redirects the browser to the
  configured `urls.logout` endpoint with a `logout_challenge` query param. The handler now has SDK
  methods to process these requests:
  - `c:get_logout_request(challenge)` — fetch the pending logout request (subject, sid, client,
    rp_initiated flag)
  - `c:accept_logout(challenge)` — invalidate the Hydra and Kratos sessions and get back the
    `redirect_to` URL pointing at the app's `post_logout_redirect_uri`
  - `c:reject_logout(challenge)` — for "stay signed in" UIs that let the user cancel the logout

  Symmetric with the existing login/consent challenge methods.

## [0.8.1] - 2026-04-07

### Fixed

- **`req.params` now URL-decodes query string values** in `http.serve`. Previously
  `?challenge=abc%3D` produced `req.params.challenge == "abc%3D"`, so consumers that re-encoded the
  value (such as `assay.hydra:get_login_request`) ended up double-encoding it to `abc%253D` and
  getting a 404 from the upstream service. Values are now decoded with `form_urlencoded::parse`, so
  `+` becomes a space and percent-escapes are decoded correctly. The raw query string remains
  available as `req.query` for handlers that need the verbatim form.

## [0.8.0] - 2026-04-07

### Added

- **Ory stack stdlib modules** — full Lua SDK for the Ory identity/authorization stack:
  - **`assay.kratos`** — Identity management. Login/registration/recovery/settings flows, identity
    CRUD via admin API, session introspection (`whoami`), schema management.
  - **`assay.hydra`** — OAuth2 and OpenID Connect. Client CRUD, authorize URL builder, token
    exchange (authorization_code grant), accept/reject login and consent challenges, token
    introspection, JWK endpoint.
  - **`assay.keto`** — Relationship-based access control. Relation-tuple CRUD, permission checks
    (Zanzibar-style), role/group membership queries, expand API for role inheritance.
  - **`assay.ory`** — Convenience wrapper that re-exports all three modules, with
    `ory.connect(opts)` to build all three clients from one options table.

  Pure Lua wrappers over the Ory REST APIs. Zero new Rust dependencies — binary size unchanged. Each
  module follows the standard `M.client(url, opts)` pattern with comprehensive `@quickref` metadata
  for `assay context` discovery.

- **Multi-value response headers in `http.serve`**: Header values can now be a Lua array of strings,
  emitting the same header name multiple times. Required for `Set-Cookie` when setting multiple
  cookies in one response, and for other headers that legitimately repeat (e.g., `Link`, `Vary`,
  `Cache-Control`).

  ```lua
  return {
    status = 200,
    headers = {
      ["Set-Cookie"] = {
        "session=abc; Path=/",
        "csrf=xyz; Path=/",
      },
    },
  }
  ```

  String values continue to work as before.

### Theme

This is the **identity and auth stack** release. Assay now ships with a complete SDK for building
OIDC-integrated apps on Ory: one app can handle Hydra login/consent challenges, query Keto
permissions, and manage Kratos identities — all in idiomatic Lua with zero external dependencies
beyond the existing assay binary.

## [0.7.2] - 2026-04-07

### Added

- **`req.params` in `http.serve`**: Query string parameters are now automatically parsed into a
  `params` table on incoming requests. For example, `?login_challenge=abc&foo=bar` becomes
  `req.params.login_challenge == "abc"` and `req.params.foo == "bar"`. The raw query string remains
  available as `req.query`.

## [0.7.1] - 2026-04-06

### Changed

- **Temporal included by default**: The `temporal` feature is now part of the default build. The
  standard Docker image and binary include native gRPC workflow support out of the box.
- **CI/Release/Docker**: Added `protoc` installation to all build environments for gRPC proto
  compilation.

## [0.7.0] - 2026-04-06

### Added

- **Temporal gRPC client** (optional `temporal` feature): Native gRPC bridge for Temporal workflow
  engine via `temporalio-client` v0.2.0. The `temporal` global provides `connect()` for persistent
  clients and `start()` for one-shot workflow execution. Client methods: `start_workflow`,
  `signal_workflow`, `query_workflow`, `describe_workflow`, `get_result`, `cancel_workflow`,
  `terminate_workflow`. All methods are async and use JSON payload encoding. Build with
  `cargo build --features temporal` — requires `protoc` (install via `mise install protoc`).
- **8 new tests** for temporal gRPC registration, error handling, and stdlib compatibility.

### Dependencies (temporal feature only)

- `temporalio-client` 0.2.0
- `temporalio-sdk` 0.2.0
- `temporalio-common` 0.2.0
- `url` 2.x

## [0.6.1] - 2026-04-06

### Fixed

- **http.serve async handlers**: Route handlers are now async (`call_async`), allowing them to call
  `http.get`, `sleep`, and any other async builtins. Previously, calling an async function from a
  route handler would crash with "attempt to yield from outside a coroutine". This was the only
  remaining sync call site for user Lua functions.

### Added

- **`npx skills add developerinlondon/assay`** — install Assay's SKILL.md into your AI agent project
  via the skills CLI.
- **Dark/light theme toggle** on assay.rs with localStorage persistence.
- **Version stamp in site footer** — shows git tag or SHA from deploy pipeline.
- **Infrastructure Testing** highlighted as core capability on the homepage.

### Changed

- **Site overhaul** — compact hero, service grid above the fold with SVG icons, side-by-side size &
  speed comparison charts, consistent nav across all pages, accurate module coverage (removed
  misleading "Coming Soon" features).
- **Comparison page** — renamed from "MCP Comparison", removed out-of-scope entries, shows only
  domains Assay actually covers.
- **README** — full size & speed comparison table with all 10 runtimes and cold start times.

## [0.6.0] - 2026-04-05

### Added

- **6 new stdlib modules** (23 -> 29 total):
  - **assay.openclaw** — OpenClaw AI agent platform integration. Invoke tools, send messages, manage
    persistent state with JSON files, diff detection, approval gates, cron jobs, sub-agent spawning,
    and LLM task execution. Auto-discovers `$OPENCLAW_URL`/`$CLAWD_URL`.
  - **assay.github** — GitHub REST API client (no `gh` CLI dependency). Pull requests (view, list,
    reviews, merge), issues (list, get, create, comment), repositories, Actions workflow runs, and
    GraphQL queries. Bearer token auth via `$GITHUB_TOKEN`.
  - **assay.gmail** — Gmail REST API client with OAuth2 token auto-refresh. Search, read, reply,
    send emails, and list labels. Uses Google OAuth2 credentials and token files.
  - **assay.gcal** — Google Calendar REST API client with OAuth2 token auto-refresh. Events CRUD
    (list, get, create, update, delete) and calendar list. Same auth pattern as gmail.
  - **assay.oauth2** — Google OAuth2 token management. File-based credentials loading, automatic
    access token refresh via refresh_token grant, token persistence, and auth header generation.
    Used internally by gmail and gcal modules. Default paths: `~/.config/gog/credentials.json` and
    `~/.config/gog/token.json`.
  - **assay.email_triage** — Email classification and triage. Deterministic rule-based
    categorization of emails into needs_reply, needs_action, and fyi buckets. Optional LLM-assisted
    triage via OpenClaw for smarter classification. Subject and sender pattern matching for
    automated mail detection.
- **Tool mode**: `assay run --mode tool` for OpenClaw integration. Runs Lua scripts as deterministic
  tools invoked by AI agents, with structured JSON output.
- **Resume mechanism**: `assay resume --token <token> --approve yes|no` for resuming paused
  workflows after human approval gates.
- **OpenClaw extension**: `@developerinlondon/assay-openclaw-extension` package (GitHub Packages).
  Registers Assay as an OpenClaw agent tool with configurable script directory, timeout, output size
  limits, and approval-based resume flow. Install via
  `openclaw plugins install @developerinlondon/assay-openclaw-extension`.

### Architecture

- **Shell-free design**: All 6 new modules use native HTTP APIs exclusively. No shell commands, no
  CLI dependencies (no `gh`, no `gcloud`, no `oauth2l`). Pure Lua over Assay HTTP builtins.

## [0.5.6] - 2026-04-03

### Added

- **SSE streaming** for `http.serve` via `{ sse = function(send) ... end }` return shape. SSE
  handler runs async so `sleep()` and other async builtins work inside the producer. `send` callback
  uses async channel send with proper backpressure handling. Custom headers take precedence over SSE
  defaults (Content-Type, Cache-Control, Connection).
- **assert.ne(a, b, msg?)** — inequality assertion for the test framework.

### Fixed

- **Content-Type precedence**: User-provided `Content-Type` header no longer overwritten by defaults
  (`text/plain` / `application/json`) in `http.serve` responses.
- **SSE newline validation**: `event` and `id` fields reject values containing newlines or carriage
  returns to prevent SSE field injection.

## [0.5.5] - 2026-03-13

### Added

- **follow_redirects** option for YAML HTTP checks. Set `follow_redirects: false` to disable
  automatic redirect following, allowing verification of auth-protected endpoints that return 302
  redirects to identity providers. Defaults to `true` for backward compatibility.
- **follow_redirects** option for Lua `http.client()` builder. Create clients with
  `http.client({ follow_redirects = false })` for the same no-redirect behavior in scripts.

## [0.5.4] - 2026-03-12

### Fixed

- **unleash.ensure_token**: Send `tokenName` instead of `username` in create token API payload. The
  Unleash API expects `tokenName` — sending `username` caused HTTP 400 (BadDataError). Function now
  accepts both `opts.tokenName` and `opts.username` for backward compatibility. Existing token
  matching also checks `t.tokenName` with fallback to `t.username`.

## [0.5.3] - 2026-03-12

### Added

- **disk builtins**: `disk.usage(path)` and `disk.mounts()` for filesystem disk information
- **os builtins**: `os.info()` returning name, version, arch, hostname, uptime
- **Expanded fs builtins**: `fs.exists`, `fs.is_dir`, `fs.is_file`, `fs.list`, `fs.mkdir`,
  `fs.remove`, `fs.rename`, `fs.copy`, `fs.stat`, `fs.glob`, `fs.temp_dir`
- **Expanded env builtins**: `env.set`, `env.unset`, `env.list`, `env.home`, `env.cwd`

### Fixed

- Cross-platform casts in `disk.rs` (`u32` on macOS, `u64` on Linux)

## [0.5.2] - 2026-03-11

### Added

- **shell builtins**: `shell.run(cmd)`, `shell.output(cmd)`, `shell.which(name)`, `shell.pipe(cmds)`
- **process builtins**: `process.spawn(cmd, opts)`, `process.kill(pid)`, `process.pid()`,
  `process.list()`, `process.sleep(secs)`
- **Expanded fs builtins**: `fs.read_bytes`, `fs.write_bytes`, `fs.append`, `fs.symlink`,
  `fs.readlink`, `fs.canonicalize`, `fs.metadata`

### Fixed

- `http.serve` port race condition — use ephemeral ports with `_SERVER_PORT` global
- Symlink safety, timeout validation, pipe drain, PID validation hardening

## [0.5.1] - 2026-02-23

### Added

- **Website**: Static site at assay.rs on Cloudflare Pages with homepage, module reference, AI agent
  integration guides, and MCP comparison page mapping 42 servers
- **llms.txt**: LLM agent context traversal files (`llms.txt` and `llms-full.txt`)
- **Enriched search keywords**: All 23 stdlib modules and builtins enriched with `@keywords`
  metadata for improved discovery

### Changed

- Updated README with website links
- Updated SKILL.md with MCP comparison and agent integration guidance

## [0.5.0] - 2026-02-23

### Added

- **CLI subcommands**: `assay exec` for inline Lua execution, `assay context` for prompt-ready
  module output, `assay modules` for listing all available modules
- **Module discovery**: LDoc metadata parser with auto-function extraction from all 23 stdlib
  modules
- **Search engine**: Zero-dependency BM25 search with FTS5 backend for `db` feature
- **Filesystem module loader**: Project/global/builtin priority for `require()` resolution
- **LDoc metadata headers**: All 23 stdlib modules annotated with `@module`, `@description`,
  `@keywords`, `@quickref`

### Changed

- CLI restructured to clap subcommands with backward compatibility
- Feature flags added for optional `db`, `server`, and `cli` dependencies

## [0.4.4] - 2026-02-20

### Added

- **Unleash stdlib module** (`assay.unleash`): Feature flag management client for Unleash. Projects
  (CRUD, list), environments (enable/disable per project), features (CRUD, archive, toggle on/off),
  strategies (list, add), API tokens (CRUD). Idempotent helpers: `ensure_project`,
  `ensure_environment`, `ensure_token`.

## [0.4.3] - 2026-02-13

### Added

- **crypto.hmac**: HMAC builtin supporting all 8 hash algorithms (SHA-224/256/384/512,
  SHA3-224/256/384/512). Binary-safe key/data via `mlua::String`. Supports `raw` output mode for key
  chaining (required by AWS Sig V4). Manual RFC 2104 implementation using existing sha2/sha3 crates
  — zero new dependencies.
- **S3 stdlib module** (`assay.s3`): Pure Lua S3 client with AWS Signature V4 request signing. Works
  with any S3-compatible endpoint (AWS, iDrive e2, Cloudflare R2, MinIO). Operations: create/delete
  bucket, list buckets, put/get/delete/list/head/copy objects, bucket_exists. Path-style URLs
  default. Epoch-to-UTC date math (no os.date dependency). Simple XML response parsing via Lua
  patterns.
- 15 new tests (7 HMAC + 8 S3 stdlib)

### Changed

- **Modular builtins**: Split monolithic `builtins.rs` (1788 lines) into `src/lua/builtins/`
  directory with 10 focused modules: http, json, serialization, assert, crypto, db, ws, template,
  core, mod. Zero behavior change — pure refactoring for maintainability.

## [0.4.2] - 2026-02-13

### Fixed

- **zitadel.find_app**: Improved with name query filter and resilient 409 conflict handling

## [0.4.1] - 2026-02-13

### Fixed

- **zitadel.create_oidc_app**: Handle 409 conflict responses gracefully

## [0.4.0] - 2026-02-13

### Added

- **Zitadel stdlib module** (`assay.zitadel`): OIDC identity management with JWT machine auth
- **Postgres stdlib module** (`assay.postgres`): Postgres-specific helpers
- **Vault enhancements**: Additional vault helper functions
- **healthcheck.wait**: Wait helper for health check polling

### Fixed

- Use merge-patch content-type in `k8s.patch`

## [0.3.3] - 2026-02-12

### Added

- **Filesystem require fallback**: External Lua libraries can be loaded via filesystem `require()`

### Fixed

- Load K8s CA cert for in-cluster HTTPS API calls

## [0.3.2] - 2026-02-11

### Added

- **crypto.jwt_sign**: `kid` (Key ID) header support for JWT signing

### Fixed

- Release workflow: Filter artifact download to exclude Docker metadata

## [0.3.1] - 2026-02-11

- Publish crate as `assay-lua` on crates.io (binary still installs as `assay`)
- Add release pipeline: pre-built binaries (Linux x86_64 static, macOS Apple Silicon), Docker,
  crates.io
- Add prerequisite docs to K8s-dependent examples
- Fix flaky sleep timing test

## [0.3.0] - 2026-02-11

First feature-complete release. Assay is now a general-purpose Lua runtime for Kubernetes — covering
verification, scripting, automation, and lightweight web services in a single ~9 MB binary.

### Added

- **Direct Lua execution**: `assay script.lua` with auto-detection by file extension
- **Shebang support**: `#!/usr/bin/assay` for executable Lua scripts
- **HTTP server**: `http.serve(port, routes)` — Lua scripts become web services
- **Database access**: `db.connect/query/execute` — PostgreSQL, MySQL/MariaDB, SQLite via sqlx
- **WebSocket client**: `ws.connect/send/recv/close` via tokio-tungstenite
- **Template engine**: `template.render/render_string` via minijinja (Jinja2-compatible)
- **Filesystem write**: `fs.write(path, content)` complements existing `fs.read`
- **YAML builtins**: `yaml.parse/encode` for YAML processing in Lua scripts
- **TOML builtins**: `toml.parse/encode` for TOML processing in Lua scripts
- **Async primitives**: `async.spawn(fn)` and `async.spawn_interval(ms, fn)` with handles
- **Crypto hash**: `crypto.hash(algo, data)` — SHA-256, SHA-384, SHA-512, SHA3-256, SHA3-512
- **Crypto random**: `crypto.random(length)` — cryptographically secure random hex strings
- **JWT signing**: `crypto.jwt_sign(claims, key, algo)` — RS256/RS384/RS512
- **Regex**: `regex.match/find/find_all/replace` via regex-lite
- **Base64**: `base64.encode/decode`
- **19 stdlib modules**: prometheus, alertmanager, loki, grafana, k8s, argocd, kargo, flux, traefik,
  vault, openbao, certmanager, eso, dex, crossplane, velero, temporal, harbor, healthcheck
- **E2E dogfood tests**: Assay testing itself via YAML check mode
- **CI**: GitHub Actions with clippy + tests on Linux (x86_64) and macOS (Apple Silicon)
- **491 tests**, 0 clippy warnings

### Changed

- CLI changed from `assay --config file.yaml` to `assay <file>` (positional arg, auto-detect)
- Lua upgraded from 5.4 to 5.5 (global declarations, incremental major GC, compact arrays)
- HTTP builtins DRYed (collapsed 4x duplicated method registrations into generic loop)

## [0.0.1] - 2026-02-09

Initial release. YAML-based check orchestration for ArgoCD PostSync verification.

### Added

- YAML config with timeout, retries, backoff, parallel execution
- Check types: `type: http`, `type: prometheus`, `type: script` (Lua)
- Built-in retry with exponential backoff
- Structured JSON output with pass/fail per check
- K8s-native exit codes (0 = all passed, 1 = any failed)
- HTTP client builtins: `http.get/post/put/patch`
- JSON builtins: `json.parse/encode`
- Assert builtins: `assert.eq/gt/lt/contains/not_nil/matches`
- Logging builtins: `log.info/warn/error`
- Environment: `env.get`, `sleep`, `time`
- Prometheus stdlib module
- Docker image: Alpine 3.21 + ~5 MB binary
