# Changelog

All notable changes to Assay are documented here.

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
