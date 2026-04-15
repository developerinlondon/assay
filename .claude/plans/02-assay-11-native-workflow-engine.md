# Plan: Assay v0.11 — Workflow Engine

## Summary

Assay v0.11 delivers three things in order:

1. **Remove Temporal** from assay (cut 5MB, 60s build time, `protoc` requirement)
2. **Build the workflow engine** — a new `assay serve` mode that starts assay as a durable workflow
   server with REST + SSE API, OAuth2 auth, built-in dashboard, and PostgreSQL/SQLite backends
3. **Add workflow client** — a Lua library (`assay.workflow`) that talks to an assay workflow server
   over HTTP, letting assay apps define and execute workflows

One binary, multiple modes. The workflow engine is a **separate crate** (`assay-workflow`) in the
same workspace — independently publishable to crates.io and embeddable by other Rust projects. The
`assay` binary depends on it and exposes it via `assay serve`. Since assay already depends on axum,
sqlx, and tokio, the engine adds near-zero binary size (shared deps compile once). In production,
the same Docker image runs as both engine and worker with different entrypoints. Future services
(auth, gateway, monitoring) can be added as additional serve options.

```
┌──────────────────────────────────────────────────────────────────────────┐
│                                                                          │
│  One binary. Multiple modes.                                             │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │  assay                                                           │    │
│  │                                                                  │    │
│  │  ┌─────────────────────────┐  ┌────────────────────────────┐    │    │
│  │  │  assay run server.lua   │  │  assay serve               │    │    │
│  │  │  assay run pipeline.lua │  │                             │    │    │
│  │  │                         │  │  Workflow server:           │    │    │
│  │  │  Lua runtime.           │  │  - REST API                │    │    │
│  │  │  Your applications.     │  │  - SSE streams             │    │    │
│  │  │                         │  │  - Dashboard               │    │    │
│  │  │  Has workflow client:   │  │  - Cron scheduler          │    │    │
│  │  │  require("assay.workflow│  │  - Timer management        │    │    │
│  │  │  ")                     │  │  - OAuth2 auth             │    │    │
│  │  │  workflow.connect(...)  │  │  - PostgreSQL / SQLite     │    │    │
│  │  │  workflow.define(...)   │  │                             │    │    │
│  │  │  workflow.listen(...)   │  │  Future serve options:     │    │    │
│  │  │                         │  │  --enable auth             │    │    │
│  │  │  Talks to assay serve   │  │  --enable gateway          │    │    │
│  │  │  over HTTP.             │  │  --enable monitor          │    │    │
│  │  └─────────────────────────┘  └────────────────────────────┘    │    │
│  │                                                                  │    │
│  │  assay run --engine pipeline.lua                                 │    │
│  │  → Dev mode: engine + worker in one process, SQLite, zero config │    │
│  │                                                                  │    │
│  └──────────────────────────────────────────────────────────────────┘    │
│                                                                          │
│  cargo install assay            ← full binary (Lua + workflow engine)     │
│  cargo install assay-workflow   ← engine crate only (for Rust embedding) │
│  ghcr.io/org/assay:0.11.0                                                │
│                                                                          │
│  Production: same image, different entrypoint                            │
│  Engine:  assay serve --backend postgres://db/assay                      │
│  Worker:  assay run pipeline.lua                                         │
│                                                                          │
└──────────────────────────────────────────────────────────────────────────┘
```

## Motivation

**Today**: assay bundles Temporal SDK crates (`temporalio-client`, `temporalio-sdk`,
`temporalio-sdk-core`, `temporalio-common`, `prost-wkt-types`) that:

- Add 5MB to the binary (11MB → 16MB)
- Add 60 seconds to build time
- Require `protoc` installed at build time
- Don't actually work as a complete workflow solution (need external Temporal cluster)
- The worker implementation has never reached production stability

**Goal**: A workflow engine that is:

- **Built into assay** — `assay serve` starts the engine, no separate binary needed
- **Language-agnostic** — any HTTP client can start workflows, execute activities, send signals
- **Equal access** — Lua apps, Python apps, Go apps all use the same REST API
- **Extensible** — `assay serve` is a service mode; future services (auth, gateway) plug in here
- **Production-grade** — durable execution, crash recovery, multi-worker, cron scheduling
- **Observable** — built-in dashboard with real-time updates, OAuth2 authentication

## What to Extract from Temporal Code (before removal)

The Temporal integration is non-functional and will be removed. `temporal_worker.rs` contains replay
and coroutine patterns that are worth reviewing as reference material when building the new engine,
but the code was never production-tested so it should be treated as a starting point for ideas, not
as proven logic to port directly.

| Component                   | File                          | Lines | Reference for                      |
| --------------------------- | ----------------------------- | ----- | ---------------------------------- |
| Coroutine workflow model    | `temporal_worker.rs:82-170`   | ~90   | `replay.rs` — core replay engine   |
| Command yield/resume        | `temporal_worker.rs:931-1144` | ~210  | `replay.rs` — command parsing      |
| Replay buffers              | `temporal_worker.rs:470-530`  | ~60   | `replay.rs` — deterministic replay |
| Activity dispatch           | `temporal_worker.rs:260-376`  | ~116  | `api/tasks.rs` — adapt to HTTP     |
| Signal/Timer/Query handling | `temporal_worker.rs:745-1153` | ~400  | `engine.rs` — same semantics       |

## Landscape Research (what exists, what to learn from)

We evaluated 6 existing workflow engines. None fit our requirements, but several have patterns worth
studying. Repos cloned at `~/forks/` for reference.

### Why We're Building Our Own

| Project                  | License | Language | Why Not                                                                                                                                    |
| ------------------------ | ------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| **Duroxide** (Microsoft) | MIT     | Rust     | Library only (no REST API, no dashboard, no Postgres, no scheduler). Closest match but alpha and missing everything above the engine core. |
| **Restate**              | BSL 1.1 | Rust     | Restrictive license. RocksDB only (no SQL). 45+ crates — massively complex.                                                                |
| **Inngest**              | SSPL    | Go       | Most restrictive license of all. Go, not Rust. Uses gRPC internally despite marketing HTTP.                                                |
| **Hatchet**              | MIT     | Go       | Go, not Rust. gRPC for worker communication. Good schema design though.                                                                    |
| **Windmill**             | AGPLv3  | Rust     | Viral license. 70+ crates. Wrong paradigm (script IDE, not workflow engine).                                                               |
| **Apalis**               | MIT     | Rust     | Task queue, not workflow engine. No event sourcing, no replay, no REST API.                                                                |

**No existing project provides**: Rust binary + Postgres/SQLite + REST+SSE + dashboard + auth +
cron + language-agnostic workers + permissive license.

### Patterns to Steal

#### From Duroxide (`~/forks/duroxide`) — Replay Engine + Provider Trait

The most architecturally relevant project. Study these specific patterns:

**1. Provider / ManagementProvider trait split** (`src/providers/mod.rs`,
`src/providers/management.rs`)

- Hot-path trait (`Provider`): `fetch_orchestration_item`, `ack_orchestration_item`,
  `enqueue_for_worker`, `fetch_work_item`, `ack_work_item`
- Cold-path trait (`ManagementProvider`): `list_instances`, `list_executions`, `read_execution`
- Apply to our design: `WorkflowStore` for engine hot-path, separate admin query methods for
  dashboard/API

**2. Replay engine** (`src/runtime/replay_engine.rs`)

- Turn-by-turn execution model with `TurnResult` enum (Continue, Completed, Failed, ContinueAsNew,
  Cancelled)
- Event correlation via `event_id` — every scheduled action gets an ID, completions match by that ID
- `history_delta` + `pending_actions` separation (new events generated this turn vs actions to
  dispatch)
- `persisted_history_len` tracking for distinguishing replay vs new execution
- Apply to our design: port the turn-based execution model into our engine, use event_id correlation

**3. SQLite provider** (`src/providers/sqlite.rs`, 4566 lines)

- Peek-lock semantics for work items (claim with lock token, renew lock, abandon)
- Delayed visibility for timers (enqueue with future visibility timestamp)
- Lock renewal for long-running activities
- Apply to our design: reference for our SqliteStore implementation, especially lock semantics

**4. Activity tags for worker routing** (`src/providers/mod.rs` TagFilter)

- `DefaultOnly`, `Tags(["gpu"])`, `DefaultAnd(["gpu"])`, `Any`, `None`
- Workers declare which tags they accept; activities are routed accordingly
- Apply to our design: maps directly to our task_queue concept, but more granular

#### From Inngest (`~/forks/inngest`) — Schema Design + Worker Tracking

**1. Dual Postgres/SQLite schema** (`pkg/cqrs/base_cqrs/sqlc/postgres/schema.sql`,
`sqlite/schema.sql`)

- Maintains separate schemas for both backends with dialect-appropriate types
- `worker_connections` table tracks: `worker_ip`, `max_worker_concurrency`, `connected_at`,
  `last_heartbeat_at`, `disconnected_at`, `sdk_lang`, `sdk_version`, `cpu_cores`, `mem_bytes`, `os`
- Apply to our design: enhance our `workflow_workers` table with SDK/platform metadata

**2. History table with idempotency** (`history` table)

- `idempotency_key` per step — prevents duplicate execution even across replays
- `step_type` enum for different kinds of history entries (activity, sleep, wait_for_event, invoke)
- `attempt` counter per history entry
- Apply to our design: add idempotency_key to our workflow_events table

**3. App registration model** (`apps` table)

- Workers register as "apps" with `sdk_language`, `sdk_version`, `framework`, `checksum`, `url`
- Engine tracks app status and errors
- Apply to our design: richer worker registration metadata in our REST API

#### From Hatchet (`~/forks/hatchet`) — Postgres Schema + Multi-tenancy

**1. Multi-tenant data model** (`cmd/hatchet-migrate/migrate/migrations/20240115180414_init.sql`)

- `User` → `Tenant` → `TenantMember` hierarchy
- Every workflow/job/step is scoped to a `tenantId`
- Apply to our design: consider namespace/tenant support for future multi-team usage

**2. Workflow versioning** (`WorkflowVersion` table)

- Workflows have versions with ordering. Runs are tied to a specific version.
- Apply to our design: add `workflow_version` column for safe code updates

**3. Cron at the DB level** (`WorkflowTriggerCronRef` table)

- Cron expressions stored per workflow trigger, with a `tickerId` for the evaluator
- Scheduled triggers with specific `triggerAt` timestamps
- Apply to our design: validates our `workflow_schedules` table approach

**4. Step-level granularity** (`Step`, `StepRun` tables)

- Workflows decomposed into Jobs → Steps, each with its own status tracking
- `StepRunStatus` has `PENDING_ASSIGNMENT` and `ASSIGNED` states between PENDING and RUNNING
- Apply to our design: consider adding an ASSIGNED state to our activity status enum

#### From Windmill (`~/forks/windmill`) — Dashboard Embedding

**1. Embedded frontend in Rust binary**

- Svelte frontend compiled and embedded via build process
- Served by the Rust backend at root path
- Apply to our design: same pattern for our dashboard (HTML/JS/CSS embedded via `include_dir`)

## Architecture

### System Overview

```
┌────────────────────────────────────────────────────────────────────────┐
│                                                                        │
│  YOUR APPLICATIONS (any language, any framework)                       │
│                                                                        │
│  ┌────────────────┐  ┌────────────────┐  ┌────────────────────────┐    │
│  │ my-cool-       │  │ deploy-bot     │  │ order-processor        │    │
│  │ pipeline       │  │                │  │                        │    │
│  │ (assay/Lua)    │  │ (assay/Lua)    │  │ (Go service)           │    │
│  │                │  │                │  │                        │    │
│  │ Defines:       │  │ Defines:       │  │ Calls REST API to:     │    │
│  │  IngestData wf │  │  DeployService │  │  start workflows       │    │
│  │  fetch_s3 act  │  │  provision act │  │  send signals          │    │
│  │  transform act │  │  smoke_test    │  │  query state           │    │
│  │                │  │  act           │  │                        │    │
│  │ Listens on:    │  │ Listens on:    │  │ (not a worker —        │    │
│  │  queue: data   │  │  queue: deploy │  │  just a client)        │    │
│  └───────┬────────┘  └───────┬────────┘  └───────────┬────────────┘    │
│          │                   │                       │                 │
│  ┌───────┴────────┐  ┌───────┴──────┐  ┌─────────────┴───────────────┐ │
│  │ ml-trainer     │  │ dashboard    │  │ CLI                         │ │
│  │ (Python)       │  │ (browser)    │  │                             │ │
│  │                │  │              │  │ $ assay workflow list        │ │
│  │ Polls for      │  │ Views wfs,   │  │ $ assay schedule create ... │ │
│  │ activities on  │  │ events,      │  │                             │ │
│  │ queue: gpu     │  │ schedules    │  │                             │ │
│  └───────┬────────┘  └──────┬───────┘  └──────────────┬──────────────┘ │
│          │                  │                         │                │
│          └──────────────────┼─────────────────────────┘                │
│                             │                                          │
│                   ALL use the same REST + SSE API                      │
│                             │                                          │
│                             ▼                                          │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │                                                                  │  │
│  │  ASSAY SERVE (:8080)                                              │  │
│  │  Same binary, service mode.                                      │  │
│  │  Deployed once. Shared infrastructure.                           │  │
│  │                                                                  │  │
│  │  ┌──────────────────────────────────────────────────────────┐   │  │
│  │  │  REST API + SSE                                          │   │  │
│  │  │  (axum)                                                  │   │  │
│  │  │                                                          │   │  │
│  │  │  Workflow Management    Task Execution    Real-time      │   │  │
│  │  │  POST /workflows       POST /register    GET /events    │   │  │
│  │  │  GET  /workflows       GET  /tasks/stream GET /wf/sse   │   │  │
│  │  │  POST /signal          POST /complete                   │   │  │
│  │  │  GET  /query           POST /heartbeat                  │   │  │
│  │  └──────────────────────────────────────────────────────────┘   │  │
│  │                                                                  │  │
│  │  ┌─────────────┐ ┌─────────────┐ ┌──────────────────────────┐  │  │
│  │  │ Cron        │ │ Timer       │ │ Health Monitor           │  │  │
│  │  │ Scheduler   │ │ Poller      │ │                          │  │  │
│  │  │             │ │             │ │ Detect dead workers,     │  │  │
│  │  │ Evaluate    │ │ Fire due    │ │ release their claimed    │  │  │
│  │  │ expressions,│ │ timers,     │ │ tasks back to pending.   │  │  │
│  │  │ start runs  │ │ record      │ │                          │  │  │
│  │  │             │ │ events      │ │ Timeout stale activities │  │  │
│  │  └──────┬──────┘ └──────┬──────┘ └────────────┬─────────────┘  │  │
│  │         │               │                      │                │  │
│  │  ┌──────▼───────────────▼──────────────────────▼──────────────┐ │  │
│  │  │  WorkflowStore trait                                       │ │  │
│  │  │  (only component that touches the database)                │ │  │
│  │  │                                                            │ │  │
│  │  │  ┌──────────────────┐  ┌──────────────────┐               │ │  │
│  │  │  │  PostgresStore   │  │  SqliteStore      │               │ │  │
│  │  │  │  (multi-instance)│  │  (single-instance)│               │ │  │
│  │  │  └────────┬─────────┘  └────────┬──────────┘               │ │  │
│  │  └───────────┼──────────────────────┼─────────────────────────┘ │  │
│  │              │                      │                           │  │
│  └──────────────┼──────────────────────┼───────────────────────────┘  │
│                 ▼                      ▼                               │
│       ┌──────────────┐       ┌──────────────┐                        │
│       │  PostgreSQL   │  OR  │    SQLite     │                        │
│       └──────────────┘       └──────────────┘                        │
│                                                                       │
└───────────────────────────────────────────────────────────────────────┘
```

### Production Deployment (Kubernetes)

```
┌─────────────────────────────────────────────────────────────────────┐
│  Kubernetes Cluster                                                  │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │  infra namespace                                              │  │
│  │                                                               │  │
│  │  ┌─────────────────────────────────────────────────────────┐  │  │
│  │  │  Deployment: assay-server                               │  │  │
│  │  │  replicas: 2 (HA)                                       │  │  │
│  │  │  image: ghcr.io/org/assay:0.11.0                        │  │  │
│  │  │                                                         │  │  │
│  │  │  assay serve \                                          │  │  │
│  │  │    --backend postgres://db:5432/assay \                 │  │  │
│  │  │    --port 8080 \                                        │  │  │
│  │  │    --auth-issuer https://hydra.internal \               │  │  │
│  │  │    --auth-client-id assay-workflow                       │  │  │
│  │  │                                                         │  │  │
│  │  │  Cron scheduler: pg_advisory_lock ensures only one      │  │  │
│  │  │  replica evaluates crons (automatic leader election).   │  │  │
│  │  │                                                         │  │  │
│  │  │  Service: assay-server:8080 (ClusterIP)                 │  │  │
│  │  │  Ingress: assay.yourcompany.com → :8080 (dashboard)     │  │  │
│  │  └─────────────────────────────────────────────────────────┘  │  │
│  │                                                               │  │
│  │  ┌──────────────────────────────────────────────────────┐    │  │
│  │  │  StatefulSet: postgresql                              │    │  │
│  │  │  (or RDS / CloudSQL / Neon / Supabase)                │    │  │
│  │  │                                                       │    │  │
│  │  │  Network Policy: only assay-server can connect        │    │  │
│  │  └──────────────────────────────────────────────────────┘    │  │
│  │                                                               │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │  apps namespace                                               │  │
│  │                                                               │  │
│  │  ┌───────────────────────┐  ┌───────────────────────┐        │  │
│  │  │ my-cool-pipeline      │  │ deploy-bot            │        │  │
│  │  │ replicas: 3           │  │ replicas: 2           │        │  │
│  │  │ image: my-pipeline:v2 │  │ image: deploy-bot:v1  │        │  │
│  │  │                       │  │                       │        │  │
│  │  │ assay run pipeline.lua│  │ assay run deploy.lua  │        │  │
│  │  │                       │  │                       │        │  │
│  │  │ Connects to:          │  │ Connects to:          │        │  │
│  │  │ http://assay-server   │  │ http://assay-server   │        │  │
│  │  │ .infra:8080           │  │ .infra:8080           │        │  │
│  │  └───────────────────────┘  └───────────────────────┘        │  │
│  │                                                               │  │
│  │  ┌───────────────────────┐  ┌───────────────────────┐        │  │
│  │  │ ml-trainer (Python)   │  │ order-processor (Go)  │        │  │
│  │  │ replicas: 2 (GPU)     │  │ replicas: 5           │        │  │
│  │  │                       │  │                       │        │  │
│  │  │ Polls REST API for    │  │ Starts workflows via  │        │  │
│  │  │ activities on gpu q   │  │ POST /api/v1/workflows│        │  │
│  │  └───────────────────────┘  └───────────────────────┘        │  │
│  │                                                               │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### Client ↔ Engine Communication

```
┌─────────────────────────────────────────────────────────────────────┐
│                                                                     │
│  REST + SSE — One protocol, everyone equal                          │
│                                                                     │
│                                                                     │
│  WORKER APP (my-cool-pipeline) REGISTERING + LISTENING              │
│  ═══════════════════════════════════════════════════                 │
│                                                                     │
│  App                                     Engine                     │
│   │                                        │                        │
│   │  POST /api/v1/workers/register         │                        │
│   │  { identity: "pipeline-pod-1",         │                        │
│   │    queue: "data",                      │                        │
│   │    workflows: ["IngestData"],          │                        │
│   │    activities: ["fetch_s3", ...] }     │                        │
│   │ ──────────────────────────────────────→│                        │
│   │                                        │                        │
│   │  200 OK { worker_id: "w-abc" }         │                        │
│   │ ←──────────────────────────────────────│                        │
│   │                                        │                        │
│   │  GET /api/v1/tasks/stream              │                        │
│   │    ?queue=data&worker_id=w-abc         │                        │
│   │  Accept: text/event-stream             │                        │
│   │ ──────────────────────────────────────→│                        │
│   │                                        │                        │
│   │  (SSE connection open — engine pushes  │                        │
│   │   tasks as they become available)      │                        │
│   │                                        │                        │
│   │  event: task                           │                        │
│   │  data: { "task_id": "t-001",           │                        │
│   │    "type": "workflow",                 │                        │
│   │    "workflow_type": "IngestData",      │                        │
│   │    "events": [...],                    │                        │
│   │    "input": {...} }                    │                        │
│   │ ←──────────────────────────────────────│                        │
│   │                                        │                        │
│   │  (app replays events, runs Lua,        │                        │
│   │   yields commands)                     │                        │
│   │                                        │                        │
│   │  POST /api/v1/tasks/t-001/complete     │                        │
│   │  { "commands": [                       │                        │
│   │    { "ScheduleActivity":               │                        │
│   │      { "name": "fetch_s3", ... } }     │                        │
│   │  ] }                                   │                        │
│   │ ──────────────────────────────────────→│                        │
│   │                                        │                        │
│   │  (engine persists events, schedules    │                        │
│   │   activity, pushes next task via SSE)  │                        │
│   │                                        │                        │
│   │  event: task                           │                        │
│   │  data: { "task_id": "t-002",           │                        │
│   │    "type": "activity",                 │                        │
│   │    "name": "fetch_s3",                 │                        │
│   │    "input": {...} }                    │                        │
│   │ ←──────────────────────────────────────│                        │
│   │                                        │                        │
│   │  POST /api/v1/tasks/t-002/heartbeat    │                        │
│   │  { "details": { "progress": "50%" } }  │                        │
│   │ ──────────────────────────────────────→│  (during long tasks)   │
│   │                                        │                        │
│   │  POST /api/v1/tasks/t-002/complete     │                        │
│   │  { "result": { "rows": 42 } }         │                        │
│   │ ──────────────────────────────────────→│                        │
│   │                                        │                        │
│   │  (engine resumes workflow, pushes      │                        │
│   │   next task via same SSE stream...)    │                        │
│                                                                     │
│                                                                     │
│  EXTERNAL APP (Go service) STARTING A WORKFLOW                      │
│  ═════════════════════════════════════════════                       │
│                                                                     │
│  Go App                                  Engine                     │
│   │                                        │                        │
│   │  POST /api/v1/workflows                │                        │
│   │  { "workflow_type": "IngestData",      │                        │
│   │    "workflow_id": "ingest-batch-45",   │                        │
│   │    "input": { "source": "s3://..." } } │                        │
│   │ ──────────────────────────────────────→│                        │
│   │                                        │                        │
│   │  201 Created                           │                        │
│   │  { "workflow_id": "ingest-batch-45",   │                        │
│   │    "run_id": "r-xyz",                  │                        │
│   │    "status": "RUNNING" }               │                        │
│   │ ←──────────────────────────────────────│                        │
│   │                                        │                        │
│   │  GET /api/v1/workflows/                │                        │
│   │    ingest-batch-45/events/stream       │                        │
│   │  Accept: text/event-stream             │                        │
│   │ ──────────────────────────────────────→│                        │
│   │                                        │                        │
│   │  event: ActivityCompleted              │                        │
│   │  data: { "name": "fetch_s3", ... }     │                        │
│   │ ←──────────────────────────────────────│                        │
│   │                                        │                        │
│   │  event: WorkflowCompleted              │                        │
│   │  data: { "result": { "rows": 42 } }   │                        │
│   │ ←──────────────────────────────────────│                        │
│                                                                     │
│                                                                     │
│  DASHBOARD (browser) — REAL-TIME VIA SSE                            │
│  ═══════════════════════════════════════                             │
│                                                                     │
│  Browser                                 Engine                     │
│   │                                        │                        │
│   │  GET /workflow/                        │                        │
│   │  Cookie: session=xxx                   │                        │
│   │ ──────────────────────────────────────→│  Validate OAuth2       │
│   │                                        │  session               │
│   │  200 OK (HTML dashboard)               │                        │
│   │ ←──────────────────────────────────────│                        │
│   │                                        │                        │
│   │  JS: new EventSource(                  │                        │
│   │    "/api/v1/events/stream")            │                        │
│   │ ──────────────────────────────────────→│                        │
│   │                                        │                        │
│   │  event: WorkflowStarted               │                        │
│   │  data: { "id": "ingest-45", ... }     │  (table row appears)   │
│   │ ←──────────────────────────────────────│                        │
│   │                                        │                        │
│   │  event: ActivityCompleted              │                        │
│   │  data: { "workflow_id": "ingest-45" }  │  (status updates)      │
│   │ ←──────────────────────────────────────│                        │
│   │                                        │                        │
│   │  JS: fetch("/api/v1/workflows/         │                        │
│   │    deploy-abc/signal/approval",        │                        │
│   │    { method: "POST", body: ... })      │                        │
│   │ ──────────────────────────────────────→│  (send signal from UI) │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### Authentication (Provider-Agnostic)

Three auth modes, simplest to most secure. The engine doesn't care who issued the token — it works
with any OIDC-compliant provider.

```
┌─────────────────────────────────────────────────────────────────────┐
│                                                                     │
│  THREE AUTH MODES                                                   │
│  ════════════════                                                   │
│                                                                     │
│  1. NO AUTH (default in dev)                                        │
│     assay serve --no-auth                                           │
│     All endpoints open. For local dev and trusted networks.         │
│                                                                     │
│  2. API KEYS (simple machine-to-machine)                            │
│     assay serve --auth api-key                                      │
│     Keys stored hashed in workflow DB. No OAuth2 needed.            │
│                                                                     │
│     Worker App                              Engine                  │
│      │                                        │                     │
│      │ Authorization: Bearer <api-key>        │                     │
│      │───────────────────────────────────────→│                     │
│      │                                        │  SHA256(key)        │
│      │                                        │  matches DB? ✓     │
│                                                                     │
│     CLI management:                                                 │
│     assay serve --generate-api-key            (prints key once)     │
│     assay serve --list-api-keys               (shows hashed keys)   │
│     assay serve --revoke-api-key <prefix>     (deletes by prefix)   │
│                                                                     │
│  3. JWT/OIDC (any provider)                                         │
│     assay serve --auth-issuer https://your-provider.com             │
│     Engine fetches JWKS from {issuer}/.well-known/openid-config.    │
│     Validates signature, expiry, issuer, audience.                  │
│     Works with Ory Hydra, Keycloak, Auth0, Azure AD, Google, etc.  │
│                                                                     │
│     Worker App        Any OIDC Provider          Engine             │
│      │                      │                      │                │
│      │ POST /oauth2/token   │                      │                │
│      │ (client_credentials) │                      │                │
│      │─────────────────────→│                      │                │
│      │                      │                      │                │
│      │ { access_token: jwt }│                      │                │
│      │←─────────────────────│                      │                │
│      │                                             │                │
│      │ Authorization: Bearer <jwt>                 │                │
│      │────────────────────────────────────────────→│                │
│      │                                             │                │
│      │                         Validate JWT via JWKS (cached)       │
│      │                         Check exp, iss, aud                  │
│                                                                     │
│                                                                     │
│  DASHBOARD (humans) — OAuth2 Authorization Code Flow                │
│  ═══════════════════════════════════════════════════                 │
│  Only when --auth-issuer is set. Engine acts as an OAuth2 client.   │
│                                                                     │
│  Browser          Engine               OIDC Provider                │
│   │                 │                       │                       │
│   │ GET /workflow/  │                       │                       │
│   │ (no session)    │                       │                       │
│   │────────────────→│                       │                       │
│   │                 │                       │                       │
│   │ 302 → provider /authorize              │                       │
│   │←────────────────│                       │                       │
│   │                                         │                       │
│   │ User logs in at provider                │                       │
│   │────────────────────────────────────────→│                       │
│   │                                         │                       │
│   │ Auth code redirect                      │                       │
│   │←────────────────────────────────────────│                       │
│   │                                         │                       │
│   │ GET /auth/callback?code=xxx             │                       │
│   │────────────────→│                       │                       │
│   │                 │ Exchange code → token  │                       │
│   │                 │──────────────────────→│                       │
│   │                 │←──────────────────────│                       │
│   │                 │                       │                       │
│   │ Set-Cookie: session=jwt                 │                       │
│   │ 302 → /workflow/                        │                       │
│   │←────────────────│                       │                       │
│                                                                     │
│                                                                     │
│  TESTING                                                            │
│  ═══════                                                            │
│                                                                     │
│  No auth:  trivial — no headers needed                              │
│  API keys: generate key, pass as Bearer token                       │
│  JWT:      self-signed JWTs via crypto.jwt_sign (already in assay)  │
│            Tests generate RSA keypair, sign tokens, engine validates │
│            against in-memory JWKS. No running OAuth2 server needed. │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### Why SSE, Not WebSocket

```
┌─────────────────────────────────────────────────────────────────────┐
│                                                                     │
│  What we need         SSE              WebSocket                    │
│  ─────────────────    ───              ─────────                    │
│  Engine pushes tasks  ✓ Native         ✓ Works                      │
│  Worker posts results POST (separate)  ✓ Same conn                  │
│  Auto-reconnect       ✓ Built-in       ✗ Build yourself             │
│  Proxy/LB compat      ✓ Just HTTP      ✗ Needs upgrade config       │
│  Browser native       ✓ EventSource    ✓ WebSocket API              │
│  Complexity           Low              Medium                       │
│                                                                     │
│  HTTP/2 multiplexes POSTs over the same TCP connection.             │
│  Workers POST once every seconds-to-minutes (activity duration).    │
│  SSE covers everything. WebSocket reserved for future if needed.    │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### SSE Client in Assay (`http.get` enhancement)

`http.get` currently buffers the full response body. For SSE endpoints, the body never "ends" — it's
an open stream. Assay v0.11 adds SSE client support via auto-detection:

```
┌─────────────────────────────────────────────────────────────────────┐
│                                                                     │
│  AUTO-DETECTION                                                     │
│  ═══════════════                                                    │
│                                                                     │
│  http.get(url) checks the response Content-Type header:            │
│                                                                     │
│    text/event-stream  →  stream mode (new behavior)                │
│    anything else      →  buffer mode (existing behavior, unchanged) │
│                                                                     │
│  No new function. Same http.get. Automatic.                         │
│                                                                     │
│                                                                     │
│  API                                                                │
│  ═══                                                                │
│                                                                     │
│  -- Normal response (unchanged, 99% of calls)                      │
│  local resp = http.get("http://grafana:3000/api/health")            │
│  -- resp = { status = 200, body = "...", headers = {...} }         │
│                                                                     │
│  -- SSE stream (auto-detected from Content-Type)                   │
│  local resp = http.get("http://engine:8080/tasks/stream", {        │
│      on_event = function(event)                                     │
│          -- event = { event = "task", data = "..." }               │
│          -- called for each SSE event, runs in async context        │
│      end                                                            │
│  })                                                                 │
│  -- resp = { status = 200, headers = {...} }                       │
│  -- connection stays open until on_event returns "close"            │
│  -- or the server closes it                                         │
│                                                                     │
│                                                                     │
│  BENEFITS BEYOND WORKFLOWS                                          │
│  ═══════════════════════                                            │
│                                                                     │
│  assay.k8s:   watch pods/deployments (kube-api watch endpoints)    │
│  assay.loki:  tail logs in real-time (LogQL tail)                  │
│  assay.prometheus: stream query results                             │
│  Custom scripts: consume any SSE API                                │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

## REST API Specification

```
┌─────────────────────────────────────────────────────────────────────┐
│                                                                     │
│  ASSAY-ENGINE REST API v1                                           │
│                                                                     │
│  All endpoints: /api/v1/...                                         │
│  Auth: Bearer JWT (from Hydra) on all endpoints                     │
│  Content-Type: application/json                                     │
│                                                                     │
│                                                                     │
│  WORKFLOW MANAGEMENT                                                │
│  ═══════════════════                                                │
│                                                                     │
│  POST   /api/v1/workflows                    Start a workflow       │
│  GET    /api/v1/workflows                    List workflows         │
│  GET    /api/v1/workflows/:id                Describe workflow      │
│  GET    /api/v1/workflows/:id/events         Get event history      │
│  POST   /api/v1/workflows/:id/signal/:name   Send signal           │
│  GET    /api/v1/workflows/:id/query/:name    Run query              │
│  POST   /api/v1/workflows/:id/cancel         Cancel                │
│  POST   /api/v1/workflows/:id/terminate      Terminate             │
│                                                                     │
│  SCHEDULES                                                          │
│  ═════════                                                          │
│                                                                     │
│  POST   /api/v1/schedules                    Create schedule        │
│  GET    /api/v1/schedules                    List schedules         │
│  GET    /api/v1/schedules/:name              Describe               │
│  PATCH  /api/v1/schedules/:name              Update/pause/resume    │
│  DELETE /api/v1/schedules/:name              Delete                 │
│                                                                     │
│  TASK EXECUTION (used by worker apps, open to any client)           │
│  ════════════════════════════════════════════════════════            │
│                                                                     │
│  POST   /api/v1/workers/register             Register as worker     │
│  POST   /api/v1/workers/heartbeat            Worker heartbeat       │
│  GET    /api/v1/tasks/stream                 SSE task stream        │
│  POST   /api/v1/tasks/:id/complete           Complete a task        │
│  POST   /api/v1/tasks/:id/fail               Fail a task            │
│  POST   /api/v1/tasks/:id/heartbeat          Activity heartbeat     │
│                                                                     │
│  REAL-TIME EVENTS                                                   │
│  ════════════════                                                   │
│                                                                     │
│  GET    /api/v1/events/stream                SSE — all events       │
│  GET    /api/v1/workflows/:id/events/stream  SSE — one workflow     │
│                                                                     │
│  WORKERS & HEALTH                                                   │
│  ════════════════                                                   │
│                                                                     │
│  GET    /api/v1/workers                      List active workers    │
│  GET    /api/v1/health                       Engine health check    │
│                                                                     │
│  DASHBOARD                                                          │
│  ═════════                                                          │
│                                                                     │
│  GET    /workflow/                   Dashboard UI (HTML/JS/CSS)      │
│  GET    /auth/login                 Initiate OAuth2 flow            │
│  GET    /auth/callback              OAuth2 callback                 │
│  POST   /auth/logout                End session                     │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

## Database Schema

```sql
-- Assay-Engine Workflow Schema
-- Compatible with PostgreSQL and SQLite

CREATE TABLE IF NOT EXISTS workflows (
    id              TEXT PRIMARY KEY,
    run_id          TEXT NOT NULL,
    workflow_type   TEXT NOT NULL,
    task_queue      TEXT NOT NULL DEFAULT 'default',
    status          TEXT NOT NULL DEFAULT 'PENDING',
    -- PENDING | RUNNING | WAITING | COMPLETED | FAILED | CANCELLED | TIMED_OUT
    input           TEXT,           -- JSON
    result          TEXT,           -- JSON
    error           TEXT,
    parent_id       TEXT,           -- child workflows
    claimed_by      TEXT,           -- worker_id
    created_at      DOUBLE PRECISION NOT NULL,
    updated_at      DOUBLE PRECISION NOT NULL,
    completed_at    DOUBLE PRECISION
);
CREATE INDEX idx_wf_status_queue ON workflows(status, task_queue);

CREATE TABLE IF NOT EXISTS workflow_events (
    id              BIGSERIAL PRIMARY KEY,  -- SQLite: INTEGER PRIMARY KEY AUTOINCREMENT
    workflow_id     TEXT NOT NULL REFERENCES workflows(id),
    seq             INTEGER NOT NULL,
    event_type      TEXT NOT NULL,
    -- WorkflowStarted, ActivityScheduled, ActivityCompleted, ActivityFailed,
    -- TimerStarted, TimerFired, SignalReceived, WorkflowCompleted,
    -- WorkflowFailed, WorkflowCancelled, ChildWorkflowStarted,
    -- ChildWorkflowCompleted, SideEffectRecorded
    payload         TEXT,           -- JSON
    timestamp       DOUBLE PRECISION NOT NULL
);
CREATE INDEX idx_wf_events_lookup ON workflow_events(workflow_id, seq);

CREATE TABLE IF NOT EXISTS workflow_activities (
    id              BIGSERIAL PRIMARY KEY,
    workflow_id     TEXT NOT NULL REFERENCES workflows(id),
    seq             INTEGER NOT NULL,
    name            TEXT NOT NULL,
    task_queue      TEXT NOT NULL DEFAULT 'default',
    input           TEXT,           -- JSON
    status          TEXT NOT NULL DEFAULT 'PENDING',
    -- PENDING | RUNNING | COMPLETED | FAILED | CANCELLED
    result          TEXT,
    error           TEXT,
    attempt         INTEGER NOT NULL DEFAULT 1,
    max_attempts    INTEGER NOT NULL DEFAULT 3,
    initial_interval_secs   DOUBLE PRECISION NOT NULL DEFAULT 1,
    backoff_coefficient     DOUBLE PRECISION NOT NULL DEFAULT 2,
    start_to_close_secs     DOUBLE PRECISION NOT NULL DEFAULT 300,
    heartbeat_timeout_secs  DOUBLE PRECISION,
    claimed_by      TEXT,
    scheduled_at    DOUBLE PRECISION NOT NULL,
    started_at      DOUBLE PRECISION,
    completed_at    DOUBLE PRECISION,
    last_heartbeat  DOUBLE PRECISION
);
CREATE INDEX idx_wf_act_pending ON workflow_activities(task_queue, status, scheduled_at)
    WHERE status = 'PENDING';

CREATE TABLE IF NOT EXISTS workflow_timers (
    id              BIGSERIAL PRIMARY KEY,
    workflow_id     TEXT NOT NULL REFERENCES workflows(id),
    seq             INTEGER NOT NULL,
    fire_at         DOUBLE PRECISION NOT NULL,
    fired           BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX idx_wf_timers_due ON workflow_timers(fire_at) WHERE fired = FALSE;

CREATE TABLE IF NOT EXISTS workflow_signals (
    id              BIGSERIAL PRIMARY KEY,
    workflow_id     TEXT NOT NULL REFERENCES workflows(id),
    name            TEXT NOT NULL,
    payload         TEXT,
    consumed        BOOLEAN NOT NULL DEFAULT FALSE,
    received_at     DOUBLE PRECISION NOT NULL
);
CREATE INDEX idx_wf_signals_lookup ON workflow_signals(workflow_id, name, consumed);

CREATE TABLE IF NOT EXISTS workflow_schedules (
    name            TEXT PRIMARY KEY,
    workflow_type   TEXT NOT NULL,
    cron_expr       TEXT NOT NULL,
    input           TEXT,
    task_queue      TEXT NOT NULL DEFAULT 'default',
    overlap_policy  TEXT NOT NULL DEFAULT 'skip',
    -- skip | queue | cancel_old | allow_all
    paused          BOOLEAN NOT NULL DEFAULT FALSE,
    last_run_at     DOUBLE PRECISION,
    next_run_at     DOUBLE PRECISION,
    last_workflow_id TEXT,
    created_at      DOUBLE PRECISION NOT NULL
);

CREATE TABLE IF NOT EXISTS workflow_workers (
    id              TEXT PRIMARY KEY,
    identity        TEXT NOT NULL,
    task_queue      TEXT NOT NULL,
    workflows       TEXT,           -- JSON array
    activities      TEXT,           -- JSON array
    max_concurrent_workflows  INTEGER NOT NULL DEFAULT 10,
    max_concurrent_activities INTEGER NOT NULL DEFAULT 10,
    active_tasks    INTEGER NOT NULL DEFAULT 0,
    last_heartbeat  DOUBLE PRECISION NOT NULL,
    registered_at   DOUBLE PRECISION NOT NULL
);

CREATE TABLE IF NOT EXISTS workflow_snapshots (
    workflow_id     TEXT NOT NULL REFERENCES workflows(id),
    event_seq       INTEGER NOT NULL,
    state_json      TEXT NOT NULL,
    created_at      DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (workflow_id, event_seq)
);
```

## Lua API (assay.workflow client library)

```lua
-- my-cool-pipeline/main.lua

local workflow = require("assay.workflow")

-- Connect to assay serve instance
workflow.connect("http://assay-server:8080", {
    client_id = env.get("OAUTH_CLIENT_ID"),
    client_secret = env.get("OAUTH_CLIENT_SECRET"),
    token_url = "https://hydra.internal/oauth2/token",
})

-- Define workflows (deterministic — survives crashes via replay)
workflow.define("IngestData", function(ctx, input)
    local raw = ctx:execute_activity("fetch_s3", {
        bucket = input.source,
    }, {
        start_to_close_timeout = 300,
        retry = { max_attempts = 3, initial_interval = 5, backoff_coefficient = 2 },
    })

    ctx:sleep(10)  -- durable, persisted in engine DB

    -- Route heavy work to a different queue (Python ML workers)
    local enriched = ctx:execute_activity("enrich_ml", {
        data = raw,
    }, {
        task_queue = "gpu-tasks",
        heartbeat_timeout = 60,
    })

    ctx:execute_activity("load_warehouse", { data = enriched })

    ctx:register_query("progress", function()
        return { phase = "complete", rows = enriched.count }
    end)

    return { status = "done", rows = enriched.count }
end)

-- Define activities (non-deterministic — real work)
workflow.activity("fetch_s3", function(ctx, input)
    local s3 = require("assay.s3")
    return s3.client(env.get("S3_URL")):get(input.bucket)
end)

workflow.activity("load_warehouse", function(ctx, input)
    local conn = db.connect(env.get("WAREHOUSE_URL"))
    db.execute(conn, "INSERT INTO ...", input.data)
end)

-- Start listening — this app is now a workflow participant
workflow.listen({
    identity = "my-cool-pipeline-" .. os.hostname(),
    queue = "data-pipeline",
    max_concurrent_workflows = 10,
    max_concurrent_activities = 20,
})
```

## Repo Structure

```
assay/                                 ← workspace root (Cargo.toml)
│
├── crates/
│   └── assay-workflow/                ← SEPARATE CRATE (publishable, embeddable)
│       ├── Cargo.toml                 │  deps: sqlx, tokio, serde, cron, chrono
│       │                              │  NO: mlua, lua, assay
│       └── src/
│           ├── lib.rs                 │  Public API: Engine, SqliteStore, WorkflowStore
│           ├── engine.rs              │  Engine<S> orchestrator + high-level ops
│           ├── state.rs               │  WorkflowCommand enum, state transitions
│           ├── scheduler.rs           │  Cron evaluation (cron + chrono + tokio)
│           ├── timers.rs              │  Timer polling + TimerFired events
│           ├── health.rs              │  Dead worker detection, activity timeouts
│           ├── types.rs               │  All record types + status enums
│           ├── store/
│           │   ├── mod.rs             │  WorkflowStore trait (Send futures)
│           │   ├── sqlite.rs          │  SqliteStore (full impl + schema)
│           │   └── postgres.rs        │  PostgresStore (future)
│           ├── api/                   │  (Phase 3: REST API + SSE)
│           │   ├── mod.rs             │  Axum router
│           │   ├── workflows.rs       │  /api/v1/workflows/*
│           │   ├── tasks.rs           │  /api/v1/tasks/* + SSE
│           │   ├── schedules.rs       │  /api/v1/schedules/*
│           │   ├── workers.rs         │  /api/v1/workers/*
│           │   ├── events.rs          │  /api/v1/events/* SSE
│           │   ├── auth.rs            │  JWT/OIDC/API key middleware
│           │   └── dashboard.rs       │  Static HTML/JS serving
│           └── dashboard/             │  Embedded HTML/JS/CSS
│
├── src/                               ← ASSAY BINARY (Lua runtime)
│   ├── main.rs                        │  CLI: run, serve, workflow, schedule
│   ├── lib.rs                         │  re-exports assay_workflow as workflow
│   └── lua/
│       └── builtins/
│           ├── http.rs                │  UPDATED: SSE client support (Phase 6)
│           └── ...
│
├── stdlib/
│   └── workflow.lua                   │  Pure Lua workflow client (Phase 6)
│                                      │  (uses http.*, json.*, coroutines)
│
└── tests/
    └── workflow_store.rs              │  11 tests: CRUD, claim, timers, signals
```

## Implementation Phases

### Phase 0: Remove Temporal from Assay ✅ (released as v0.11.0)

**Goal**: Clean slate. Cut 5MB, 60s build, `protoc` requirement.

| Step | Description                                         | Files                                 |
| ---- | --------------------------------------------------- | ------------------------------------- |
| 0.1  | Remove `temporal` from default features             | `Cargo.toml`                          |
| 0.2  | Remove `temporal.rs` builtin                        | `src/lua/builtins/temporal.rs`        |
| 0.3  | Remove `temporal_worker.rs` builtin                 | `src/lua/builtins/temporal_worker.rs` |
| 0.4  | Remove temporal registration from `builtins/mod.rs` | `src/lua/builtins/mod.rs`             |
| 0.5  | Remove temporal dependencies from `Cargo.toml`      | `Cargo.toml`                          |
| 0.6  | Remove temporal tests                               | `tests/temporal_*.rs`                 |
| 0.7  | Update stdlib temporal module (deprecation notice)  | `stdlib/temporal.lua`                 |
| 0.8  | Update CHANGELOG                                    | `CHANGELOG.md`                        |

**Delivers**: Clean 11MB assay binary, fast builds, no `protoc`.

### Phase 1: Workflow Engine Scaffolding + Store — ~600 lines Rust ✅

**Goal**: Separate `assay-workflow` crate, database layer, `assay serve` subcommand.

| Step | Description                                                  | Files                                         | Status   |
| ---- | ------------------------------------------------------------ | --------------------------------------------- | -------- |
| 1.1  | Create `crates/assay-workflow/` workspace member             | `crates/assay-workflow/Cargo.toml`            | ✅       |
| 1.2  | Define types (WorkflowRecord, Event, Activity, Timer, etc.)  | `crates/assay-workflow/src/types.rs`          | ✅       |
| 1.3  | Define `WorkflowStore` trait (Send futures for tokio::spawn) | `crates/assay-workflow/src/store/mod.rs`      | ✅       |
| 1.4  | Implement `SqliteStore` (schema migration + full trait impl) | `crates/assay-workflow/src/store/sqlite.rs`   | ✅       |
| 1.5  | Implement `PostgresStore`                                    | `crates/assay-workflow/src/store/postgres.rs` | deferred |
| 1.6  | Add `assay serve`, `assay workflow`, `assay schedule` CLI    | `src/main.rs`                                 | ✅       |
| 1.7  | Unit tests for SqliteStore (11 tests)                        | `tests/workflow_store.rs`                     | ✅       |

### Phase 2: Engine Core — ~600 lines Rust ✅

**Goal**: Scheduler, timer poller, health monitor, state machine.

| Step | Description                                               | Files          | Status |
| ---- | --------------------------------------------------------- | -------------- | ------ |
| 2.1  | Workflow state machine (transitions, validation)          | `state.rs`     | ✅     |
| 2.2  | Cron scheduler (cron + chrono + tokio background task)    | `scheduler.rs` | ✅     |
| 2.3  | Timer poller (fire due timers, write events)              | `timers.rs`    | ✅     |
| 2.4  | Health monitor (dead worker detection, task reassignment) | `health.rs`    | ✅     |
| 2.5  | Engine orchestrator (wires everything together)           | `engine.rs`    | ✅     |

### Phase 3: REST API + SSE — ~800 lines Rust ✅

**Goal**: Complete HTTP API. All endpoints, SSE streams.

| Step | Description                                                            | Files              |
| ---- | ---------------------------------------------------------------------- | ------------------ |
| 3.1  | Axum router + middleware                                               | `api/mod.rs`       |
| 3.2  | Workflow CRUD endpoints                                                | `api/workflows.rs` |
| 3.3  | Task execution endpoints (register, stream, complete, fail, heartbeat) | `api/tasks.rs`     |
| 3.4  | SSE streams (all events, per-workflow, task streams)                   | `api/events.rs`    |
| 3.5  | Schedule CRUD endpoints                                                | `api/schedules.rs` |
| 3.6  | Worker registry + health endpoints                                     | `api/workers.rs`   |
| 3.7  | CLI management commands (`assay workflow list`, etc.)                  | `main.rs`          |
| 3.8  | Integration tests                                                      | tests              |

### Phase 4: Authentication — ~400 lines Rust ✅

**Goal**: Provider-agnostic auth — no auth, API keys, or JWT/OIDC (any provider).

| Step | Description                                                      | Files         |
| ---- | ---------------------------------------------------------------- | ------------- |
| 4.1  | Auth middleware (extracts Bearer token, routes to mode)          | `api/auth.rs` |
| 4.2  | API key mode (SHA256 hash lookup in workflow DB)                 | `api/auth.rs` |
| 4.3  | JWT/OIDC mode (JWKS fetch + cache, validate sig/exp/iss/aud)     | `api/auth.rs` |
| 4.4  | OAuth2 authorization code flow (dashboard login, any provider)   | `api/auth.rs` |
| 4.5  | Session management (cookies for dashboard)                       | `api/auth.rs` |
| 4.6  | No-auth mode (default in dev, `--no-auth` flag)                  | `api/auth.rs` |
| 4.7  | API key CLI (`--generate-api-key`, `--revoke-api-key`)           | `main.rs`     |
| 4.8  | Tests (self-signed JWTs via crypto.jwt_sign, no external server) | tests         |

### Phase 5: Dashboard — ~500 lines HTML/JS/CSS + ~200 lines Rust ✅

**Goal**: Built-in web UI with real-time updates.

| Step | Description                                          | Files              |
| ---- | ---------------------------------------------------- | ------------------ |
| 5.1  | Dashboard HTML/JS/CSS (embedded via `include_dir`)   | `dashboard/`       |
| 5.2  | Workflow list view (live table via SSE)              | `dashboard/`       |
| 5.3  | Workflow detail view (event timeline, signal/cancel) | `dashboard/`       |
| 5.4  | Schedule management view                             | `dashboard/`       |
| 5.5  | Worker status view                                   | `dashboard/`       |
| 5.6  | Static file serving                                  | `api/dashboard.rs` |

### Phase 6: Assay Integration — ~200 lines Rust + ~400 lines Lua ✅

**Goal**: SSE client support in assay, pure Lua workflow client library.

| Step | Description                                                          | Files                      |
| ---- | -------------------------------------------------------------------- | -------------------------- |
| 6.1  | SSE client support in `http.get` (auto-detect `text/event-stream`)   | `src/lua/builtins/http.rs` |
| 6.2  | `assay.workflow` stdlib module (pure Lua: connect, define, activity, | `stdlib/workflow.lua`      |
|      | listen — uses `http.*`, `json.*`, Lua coroutines for replay)         |                            |
| 6.3  | End-to-end tests (Lua → engine → back)                               | tests                      |

### Phase 7: Child Workflows + Advanced — ~400 lines Rust ✅

**Goal**: Nested workflows, cancellation, snapshots.

| Step | Description                                  | Files  | Status |
| ---- | -------------------------------------------- | ------ | ------ |
| 7.1  | Child workflow execution                     | engine | ✅     |
| 7.2  | Cancellation propagation (parent → children) | engine | ✅     |
| 7.3  | Continue-as-new                              | engine | ✅     |
| 7.4  | State snapshots (fast replay optimization)   | store  | ✅     |
| 7.5  | `ctx:side_effect()`                          | engine | ✅     |

## Estimated Effort

| Phase                               | Lines         | Sessions  |
| ----------------------------------- | ------------- | --------- |
| Phase 0: Remove Temporal            | -1520         | 1         |
| Phase 1: Engine scaffolding + Store | ~600          | 2         |
| Phase 2: Engine Core                | ~600          | 2-3       |
| Phase 3: REST API + SSE             | ~800          | 2-3       |
| Phase 4: Authentication             | ~400          | 1-2       |
| Phase 5: Dashboard                  | ~700          | 2-3       |
| Phase 6: Assay Integration          | ~600          | 2-3       |
| **Total**                           | **~4100 new** | **12-18** |

## CLI Summary

```bash
# ── assay serve (workflow engine mode) ──────────────────────

# Start the workflow engine (production)
assay serve --backend postgres://db/assay --port 8080
assay serve --backend sqlite:///var/lib/assay/workflows.db

# Workflow management
assay workflow list [--status RUNNING] [--type IngestData]
assay workflow describe <workflow-id>
assay workflow signal <workflow-id> <signal-name> [payload]
assay workflow cancel <workflow-id>
assay workflow terminate <workflow-id> [--reason "..."]

# Schedule management
assay schedule list
assay schedule create <name> --type IngestData --cron "0 * * * *"
assay schedule pause <name>
assay schedule resume <name>
assay schedule delete <name>

# ── assay run (Lua runtime mode) ────────────────────────────

# Run any Lua script (no workflow engine needed)
assay run server.lua

# Run a Lua app that connects to an assay serve instance
assay run pipeline.lua

# Dev mode: embedded engine + worker in one process
assay run --engine pipeline.lua
```

## Risks and Mitigations

| Risk                                    | Likelihood | Mitigation                                            |
| --------------------------------------- | ---------- | ----------------------------------------------------- |
| SSE connection limits at scale          | Low        | HTTP/2 multiplexing. Engine replicas behind LB.       |
| Cron duplicate firing (multi-replica)   | Medium     | `pg_advisory_lock` ensures single scheduler leader.   |
| Dashboard scope creep                   | High       | Ship minimal: list + detail + signal. Iterate later.  |
| Auth complexity (Ory stack setup)       | Medium     | Auth is optional. Dev mode disables it.               |
| Breaking change (removing temporal)     | Low        | Temporal never worked reliably. Clean break.          |
| SSE client in assay complicates http.rs | Low        | Auto-detect via Content-Type header. Same `http.get`. |

## Feature Comparison: Temporal vs assay serve

| Capability             | Temporal           | assay serve                    |
| ---------------------- | ------------------ | ------------------------------ |
| Durable execution      | Yes                | Yes (event sourcing + replay)  |
| Activity retry/timeout | Yes                | Yes (configurable backoff)     |
| Activity heartbeat     | Yes                | Yes                            |
| Durable timers         | Yes                | Yes (DB-persisted)             |
| Signals                | Yes                | Yes (buffered in DB)           |
| Queries                | Yes                | Yes (read-only handlers)       |
| Child workflows        | Yes                | Yes (Phase 7)                  |
| Continue-as-new        | Yes                | Yes (Phase 7)                  |
| Cron schedules         | Yes                | Yes (with overlap policies)    |
| Multi-worker           | Yes                | Yes (via REST API)             |
| Multi-language workers | Yes (SDK per lang) | Yes (any HTTP client)          |
| Web UI                 | Separate service   | Built-in                       |
| CLI                    | tctl (separate)    | `assay workflow` (same binary) |
| Authentication         | None built-in      | OAuth2 + RBAC                  |
| Real-time events       | gRPC streaming     | SSE                            |
| Deployment             | 4+ services + DB   | 1 binary + DB                  |
| Build deps             | protoc + gRPC      | None extra                     |
