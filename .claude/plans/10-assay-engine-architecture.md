> **STATUS — REV 2 (2026-04-22):** This plan is authoritative for the engine's target architecture
> (module composition, `FromRef`, trait abstractions). References to SurrealDB are obsolete —
> backends are now PG18 + SQLite only. See plan 12 Revision log for the drop rationale.

# 10 — assay-engine Architecture

Split assay into two publications: a lean scripting runtime (`assay`) and a stateful engine
(`assay-engine`) shipped as both a crate and a binary. Introduce pluggable backend traits so
workflow state, users, sessions, and Zanzibar tuples can live on PostgreSQL 18 or SQLite, selected
at runtime via `EngineConfig.backend`.

## Motivation

Assay today is a single binary: Lua runtime + stdlib + workflow engine + dashboard. Plan 11 proposes
adding a full OIDC provider and Zanzibar store. Without a structural change, a full assay binary
would grow from 9–10 MB to 25–40 MB — still small in absolute terms, but a large percentage hit for
scripting-only consumers who don't need auth.

The split solves three problems:

- **Scripting consumers** keep the small binary they use today (runtime
  - stdlib + workflow on PG/SQLite). Auth is reached over HTTP only when a script actually needs it.
- **Server consumers** (jeebon and similar) get an embeddable crate that bundles workflow + auth +
  dashboard. Pick the backend you already run.
- **Both backends ship by default in `assay-engine`.** PG18 and SQLite compile into the binary
  together; the active backend is selected at startup from `EngineConfig.backend`. The `assay`
  runtime also uses PG + SQLite only — the Lua stdlib reaches auth over HTTP regardless of backend.

## Current state

```
assay/
├── src/                      # Lua runtime + stdlib — top-level binary
├── crates/
│   └── assay-workflow/       # workflow engine (PG + SQLite via sqlx)
│       └── src/{store,api,dashboard,scheduler,dispatch_recovery,...}
```

One binary (`assay`), ~9–10 MB compressed.

## Target architecture

### Two publications per release

```
┌──────────────────────────────────────────────────────────────────┐
│                        assay release                             │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────────────────┐  ┌──────────────────────────────┐  │
│  │    assay (runtime)       │  │      assay-engine            │  │
│  │      binary only         │  │    binary + crate            │  │
│  │                          │  │                              │  │
│  │  • Lua 5.5 VM            │  │  • Workflow engine           │  │
│  │  • stdlib                │  │  • Auth (OIDC + IdP +        │  │
│  │  • Workflow engine       │  │    passkey + session +       │  │
│  │    (PG/SQLite only)      │  │    Zanzibar)                 │  │
│  │  • Dashboard             │  │  • Dashboard (full)          │  │
│  │    (workflow views)      │  │  • Backends via traits:      │  │
│  │  • CLI                   │  │    PG18 / SQLite             │  │
│  │                          │  │                              │  │
│  │  ~12–15 MB               │  │  Binary: ≤ 20 MB stripped    │  │
│  │                          │  │  Crate embed: +14–16 MB      │  │
│  │                          │  │                              │  │
│  │  Auth → HTTP to engine   │  │                              │  │
│  └──────────────────────────┘  └──────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

### Workspace layout

```
assay/ (monorepo)
├── Cargo.toml                (workspace root)
├── crates/
│   ├── assay-domain/           Shared types, errors, store traits.
│   ├── assay-workflow/       Workflow engine + WorkflowStore impls.
│   ├── assay-auth/           Auth modules + OIDC provider + Zanzibar + impls.
│   ├── assay-dashboard/      Web UI, feature-gated views (workflow / auth).
│   ├── assay-engine/         CRATE: re-exports workflow + auth + dashboard.
│   │                         BINARY: bin/assay-engine.rs standalone server.
│   └── assay/                Runtime binary + Lua stdlib.
```

Store traits live in `assay-domain`. Backend impls live alongside their domain crate
(`assay-workflow/src/store/postgres.rs`, `assay-auth/src/store/sqlite.rs`, etc.) and are gated by
Cargo features.

### Crate dependency graph

Arrows point from consumer to dependency. `assay-domain` sits at the bottom with no upward
dependencies — it's pure types + trait signatures. Domain crates (`assay-workflow`, `assay-auth`)
depend only on `assay-domain`. The engine and dashboard layer on top.

```
                  ┌──────────────────────────────────────┐
                  │           assay-domain                 │
                  │                                      │
                  │   traits: WorkflowStore              │
                  │           UserStore (0.14.0)         │
                  │           SessionStore (0.14.0)      │
                  │           ZanzibarStore (0.14.0)     │
                  │                                      │
                  │   types:  WorkflowRecord, Event,     │
                  │           Activity, Timer, Signal,   │
                  │           Schedule, Snapshot,        │
                  │           NamespaceStats, QueueStats │
                  │                                      │
                  │   (no I/O, no HTTP, no backends)     │
                  └──────────▲────────────▲──────────────┘
                             │            │
              ┌──────────────┘            └──────────────┐
              │                                          │
┌─────────────┴───────────────┐         ┌────────────────┴────────────┐
│      assay-workflow         │         │        assay-auth           │
│                             │         │       (0.14.0 scope)        │
│  impl WorkflowStore for:    │         │                             │
│    • PostgresStore  (feat)  │         │  impl UserStore for:        │
│    • SqliteStore    (feat)  │         │    • PostgresUserStore      │
│                             │         │    • SqliteUserStore        │
│  Engine, Scheduler,         │         │                             │
│  Dispatcher, Archival,      │         │  OIDC client + provider,    │
│  HTTP API (routes),         │         │  passkey, JWT, Biscuit,     │
│  dispatch_recovery          │         │  session, Zanzibar          │
└──────────────▲──────────────┘         └──────────────▲──────────────┘
               │                                       │
               └────────────────┬──────────────────────┘
                                │
             ┌──────────────────┴───────────────────┐
             │                                      │
┌────────────┴──────────────┐         ┌─────────────┴──────────────┐
│     assay-dashboard       │         │       assay-engine         │
│                           │         │    (both crate + binary)   │
│  HTML/Askama templates,   │         │                            │
│  CSS, htmx bits.          │         │  Library side:             │
│                           │         │    re-exports workflow +   │
│  feature = "workflow"     │         │    auth + dashboard +      │
│    - run list, events,    │         │    core as submodules.     │
│      timers, activities   │         │                            │
│                           │         │  Binary side (src/bin/):   │
│  feature = "auth" (0.14)  │         │    reads config, picks     │
│    - users, sessions,     │         │    backend, wires axum     │
│      Zanzibar tuples,     │         │    router, serves.         │
│      client registry      │         │                            │
└─────────────▲─────────────┘         └─────────────▲──────────────┘
              │                                     │
              └──────────────────┬──────────────────┘
                                 │
                    ┌────────────┴────────────┐
                    │          assay          │
                    │  (runtime binary, Lua)  │
                    │                         │
                    │  Lua 5.5 VM             │
                    │  stdlib (http, fs, sql, │
                    │   workflow, auth HTTP   │
                    │   wrapper)              │
                    │  CLI                    │
                    │                         │
                    │  Embeds workflow engine │
                    │  with backend-postgres  │
                    │  + backend-sqlite only. │
                    │  PG18 + SQLite backends.│
                    │                         │
                    │  Auth: HTTP wrapper     │
                    │  calls assay-engine.    │
                    └─────────────────────────┘
```

**Why `assay-domain`?** Matches the `sqlx-core` / `axum-core` convention: the crate everything
depends on, nothing depends _through_. Required because `assay-workflow` and `assay-auth` both need
shared types (user IDs, timestamps, errors) and neither should depend on the other. Keeping it
dependency-free at the bottom also means fast compile and no backend code leaks into downstream
crates that don't want it.

### Deployment shapes

The split produces two distinct binaries for two distinct use cases.

```
┌──────────────────────────────────────────────────────────────────────┐
│                        Shape A — scripting                            │
│                                                                      │
│     $ assay run my-script.lua                                        │
│                                                                      │
│   ┌─────────────────┐                                                │
│   │  assay binary   │    embedded engine (PG/SQLite)                 │
│   │                 │    workflows, events, timers persist locally   │
│   │  Lua script ────┼──► workflow.start() — in-process call          │
│   │                 │                                                │
│   │  ~12–15 MB      │                                                │
│   └─────────────────┘                                                │
│                                                                      │
│   No auth. PG18 or SQLite backend. Same footprint as today.          │
└──────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────┐
│                    Shape B — server + scripts                         │
│                                                                      │
│     $ assay-engine --config engine.toml                              │
│                                                                      │
│   ┌──────────────────────────────────┐                               │
│   │        assay-engine binary       │                               │
│   │                                  │                               │
│   │  HTTP :3000                      │                               │
│   │    /api/v1/workflows   ──►       │       ┌──────────────┐        │
│   │    /api/v1/activities            │       │  Postgres    │        │
│   │    /dashboard          ──►       ├──────►│   or SQLite  │        │
│   │    /engine/queues                │       └──────────────┘        │
│   │    /authorize  (0.14.0)          │                               │
│   │    /token      (0.14.0)          │                               │
│   │                                  │                               │
│   │  ≤ 20 MB stripped               │                               │
│   └──────────────────────────────────┘                               │
│           ▲                                                          │
│           │ HTTP/2, ~0.5–2ms localhost                               │
│           │                                                          │
│   ┌───────┴─────────┐                                                │
│   │  assay binary   │   thin Lua wrappers call over HTTP             │
│   │  (script host)  │                                                │
│   │                 │   auth.zanzibar.check(...) ──► engine          │
│   │  Lua script ────┤   workflow.signal(...)     ──► engine          │
│   │                 │                                                │
│   │  ~12–15 MB      │                                                │
│   └─────────────────┘                                                │
└──────────────────────────────────────────────────────────────────────┘
```

### Request flow

Handlers never name a specific backend. Backend is picked at `main()`, constructed once, wrapped in
`Arc<dyn WorkflowStore>`, and passed to the router. Swapping PG18 → SQLite changes one line of
config and restarts.

```
Consumer app              assay-engine binary              Backend
───────────────           ────────────────────             ────────
HTTP POST
/api/v1/workflows  ───►   axum route handler
                          (in assay-workflow crate)
                                 │
                                 │ calls trait method
                                 ▼
                          WorkflowStore::create_workflow
                          (trait in assay-domain)
                                 │
                                 │ dispatched to impl
                                 ▼
                          ┌──────────────────────┐
                          │ feature-gated at     │
                          │ compile time:        │
                          │                      │
                          │ PostgresStore  ──────┼────►  postgres
                          │ SqliteStore    ──────┼────►  sqlite file
                          └──────────────────────┘
                                 │
                                 ▼
                          returns Result<()>
                                 │
                          ◄──────┘
                          handler builds response
HTTP 201 ◄─────────────── axum writes JSON
```

### Store traits (shared)

```rust
// assay-domain/src/store.rs

pub trait WorkflowStore: Send + Sync + 'static {
    // ── Namespaces / workflows / events / activities /
    //    timers / signals / snapshots / archival /
    //    search attributes / dispatch recovery / schedules ──
    // (~50 async methods, unchanged from current trait)

    // ── Task queues & workers ──
    fn claim_workflow_task(&self, worker_id: &str, queues: &[&str])
        -> impl Future<Output = Result<Option<WorkflowTask>>> + Send;
    fn release_workflow_task(&self, task_id: &str, outcome: TaskOutcome)
        -> impl Future<Output = Result<()>> + Send;
    fn requeue_activity_for_retry(&self, activity_id: &str, next_at: f64)
        -> impl Future<Output = Result<()>> + Send;

    fn register_worker(&self, worker: &WorkflowWorker)
        -> impl Future<Output = Result<()>> + Send;
    fn heartbeat_worker(&self, id: &str, now: f64)
        -> impl Future<Output = Result<()>> + Send;
    fn list_workers(&self, namespace: &str)
        -> impl Future<Output = Result<Vec<WorkflowWorker>>> + Send;
    fn remove_dead_workers(&self, cutoff: f64)
        -> impl Future<Output = Result<Vec<String>>> + Send;

    fn get_queue_stats(&self, namespace: &str)
        -> impl Future<Output = Result<Vec<QueueStats>>> + Send;

    // ── Push subscriptions (hybrid wake-up) ──
    /// For the scheduler: workflows becoming runnable.
    fn subscribe_runnable(&self, namespace: &str)
        -> impl Stream<Item = WorkflowId> + Send;
    /// For workers: new tasks arriving on any of the listed queues.
    fn subscribe_tasks(&self, queue_names: &[&str])
        -> impl Stream<Item = WorkflowTaskId> + Send;

    // ── Leader election ──
    fn try_acquire_scheduler_lock(&self, /* ... */)
        -> impl Future<Output = Result<bool>> + Send;
}

pub trait UserStore:     Send + Sync + 'static { /* users, credentials, links */ }
pub trait SessionStore:  Send + Sync + 'static { /* sessions, JWKS history */ }
pub trait ZanzibarStore: Send + Sync + 'static {
    async fn write_tuple(&self, t: Tuple) -> Result<()>;
    async fn delete_tuple(&self, t: &Tuple) -> Result<bool>;
    async fn check(&self, object: &Object, perm: &str,
                   subject: &Subject, cons: Consistency) -> Result<CheckResult>;
    async fn expand(&self, object: &Object, perm: &str) -> Result<UsersetTree>;
    async fn lookup_resources(&self, subject: &Subject, perm: &str,
                              object_type: &str) -> Result<Vec<Object>>;
    async fn lookup_subjects(&self, object: &Object, perm: &str,
                             subject_type: &str) -> Result<Vec<Subject>>;
}
```

### Workers and task queues

Assay's workflow engine follows a Temporal-style worker/queue model. The scheduler doesn't execute
workflow code itself — it places tasks on named queues, and workers subscribed to those queues claim
and execute them.

```
┌─────────────────┐   runnable      ┌──────────────────┐
│   Scheduler     │───workflow──────│ task_queue:main  │
│  (timer heap +  │   tasks placed  │ task_queue:email │
│  subscribe_     │                 │ task_queue:heavy │
│   runnable)     │                 └──────┬───────────┘
└─────────────────┘                        │
                                           │ subscribe_tasks
                                           ▼
                            ┌──────────────────────────────┐
                            │  Workers (registered, held   │
                            │  alive by heartbeats)        │
                            │                              │
                            │   worker-1  → {main}         │
                            │   worker-2  → {main, email}  │
                            │   worker-3  → {heavy}        │
                            └──────────────────────────────┘
```

Key properties:

- **Named queues per namespace.** Routing by workload class (cpu-heavy, latency-sensitive,
  region-specific). Workers subscribe to one or more queues.
- **Worker registry with heartbeats.** `register_worker` + periodic `heartbeat_worker`; a sweeper
  calls `remove_dead_workers` on a cutoff to GC stale entries and release claimed tasks back to the
  queue.
- **Claim / release semantics.** `claim_workflow_task` atomically marks a task for a worker and sets
  a visibility timeout. `release_workflow_task` reports success or failure; released-with-failure
  tasks go back to the queue (with retry delay for activities via `requeue_activity_for_retry`).
- **Hybrid wake-up applies here too.** Workers don't poll — they use `subscribe_tasks(queue_names)`.
  Each backend implements it the same way as `subscribe_runnable`:
  - Postgres → `LISTEN assay_task_<queue>` via INSERT trigger
  - SQLite → empty stream; single-process workers use an in-memory channel
- **Leader election for the scheduler.** `try_acquire_scheduler_lock` — Postgres uses
  `pg_try_advisory_lock` (one instance wins); SQLite always returns true (single-instance). Workers
  don't need leader election — they compete on `claim_workflow_task` instead.

Queue stats (`get_queue_stats`) surface in the engine dashboard: pending depth per queue,
claimed-but-not-completed count, oldest task age, worker count. Required for diagnosing
backpressure.

### Cargo feature matrix (assay-engine)

```toml
[features]
default = [
  "workflow",
  "auth",
  "dashboard",
  "backend-postgres",
  "backend-sqlite",
]

workflow = ["assay-workflow"]
auth = ["assay-auth"]
dashboard = ["assay-dashboard"]
server = ["dep:axum", "dep:tower"] # standalone binary mode

# Both backends are ADDITIVE (not mutually exclusive) and both ship in default.
backend-postgres = ["assay-workflow/backend-postgres", "assay-auth/backend-postgres"]
backend-sqlite = ["assay-workflow/backend-sqlite", "assay-auth/backend-sqlite"]
```

Consumer examples:

```toml
# jeebon-api (embeds engine as crate, defaults — both backends compiled in)
assay-engine = "0.1"

# lean embed: workflow only, SQLite only (explicit opt-out of PG)
assay-engine = { version = "0.1", default-features = false,
                 features = ["workflow", "backend-sqlite"] }

# auth-only, Postgres (explicit opt-out of SQLite)
assay-engine = { version = "0.1", default-features = false,
                 features = ["auth", "backend-postgres"] }
```

Runtime backend selection:

```rust
// crates/assay-engine/src/main.rs
match cfg.backend {
    Backend::Postgres { url }  => run_engine::<PostgresStore>(cfg).await,
    Backend::Sqlite   { path } => run_engine::<SqliteStore>(cfg).await,
}
```

## State composition

The crate split determines _where code lives_. A second architectural decision determines _how state
flows at runtime_: each module owns a context type, and the engine composes them via axum's
`FromRef`.

### The rule

Every module crate (`assay-workflow`, `assay-auth`, future `assay-vault`, …) exports:

- One plain struct holding the module's state. Convention: `<Module>Ctx` — `WorkflowCtx`, `AuthCtx`,
  `DashboardCtx`.
- One `pub fn router() -> Router<Self::Ctx>` that returns a router statically typed on the ctx.

The engine composes:

```rust
#[derive(Clone)]
pub struct EngineState {
    pub workflow: WorkflowCtx,
    pub auth:     AuthCtx,
    pub dashboard: DashboardCtx,
}

impl FromRef<EngineState> for WorkflowCtx  { fn from_ref(s: &EngineState) -> Self { s.workflow.clone()  } }
impl FromRef<EngineState> for AuthCtx      { fn from_ref(s: &EngineState) -> Self { s.auth.clone()      } }
impl FromRef<EngineState> for DashboardCtx { fn from_ref(s: &EngineState) -> Self { s.dashboard.clone() } }

Router::new()
    .merge(assay_workflow::router())
    .merge(assay_auth::router())
    .merge(assay_dashboard::router())
    .with_state(EngineState { /* ... */ })
```

Handlers in each module use `State<WorkflowCtx>`, `State<AuthCtx>`, etc. `FromRef` does the
extraction transparently. Modules never import each other's `Ctx` types.

### No generic cascade — `Arc<dyn Trait>` for backend dispatch

`Engine<S>` becomes `Engine` with `store: Arc<dyn WorkflowStore>` inside. Handlers and module types
never name a specific backend. The runtime cost is one `Arc` bump per store call — immeasurable next
to DB round-trip latency.

### Benefits

| Concern                           | Result                                                                                                                       |
| --------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| Adding a new module (vault, etc.) | New crate with its own `Ctx`. `EngineState` gains one field + one `FromRef` impl. Zero touching of existing modules.         |
| Cross-module cycles               | None. `assay-auth` and `assay-workflow` never import each other.                                                             |
| Testing a module in isolation     | Build a mock `Ctx` with mock stores; no transitive dep on other modules.                                                     |
| Shared backend connection pool    | Engine opens one pool, hands clones to each module via `from_pool` constructors. Modules don't know or care they're sharing. |

### Backend crate layout — "Layout 1"

Backends live **inside** the domain crate, feature-gated:

```
crates/assay-workflow/src/store/
├── mod.rs
├── postgres.rs     #[cfg(feature = "backend-postgres")]
└── sqlite.rs       #[cfg(feature = "backend-sqlite")]
```

Not one crate per backend (the `sqlx-postgres` / `sqlx-sqlite` / `sqlx-mysql` approach). Reasoning:
trait evolution dominates during 0.x — a new method on `WorkflowStore` requires updating both
backend impls in lockstep. Layout 1 keeps that change in one crate, one PR, one version bump. The
`sqlx`-style split becomes valuable once the traits stabilise and third-party backend crates appear
— not a 0.13.0 concern.

## Dispatch wake-up — hybrid model (from day one)

`LISTEN/NOTIFY` alone doesn't solve dispatch because `next_dispatch_at <= now()` is a wall-clock
condition — a workflow doesn't emit an event when its dispatch time _arrives_. The scheduler always
needs time-based triggering; push notifications are an optimisation that avoids waking it for
nothing when nothing has changed.

The design from day one, baked into `WorkflowStore::subscribe_runnable`:

```
┌──────────────────────────────────────────────────────────────────────┐
│                           Scheduler                                  │
│                                                                      │
│    Local min-heap of (next_dispatch_at, workflow_id)                 │
│    OS timer wakes when the earliest fires → dispatch(ws)             │
│                                                                      │
│    Backend push stream wakes the scheduler EARLY when:               │
│      - a workflow is newly inserted with status='runnable'           │
│      - a workflow transitions to status='runnable' (timer, signal)   │
│      - next_dispatch_at is updated to a sooner time                  │
│                                                                      │
│    On early-wake: re-read affected rows, update heap, redispatch.    │
└──────────────────────────────────────────────────────────────────────┘
```

Backend impls of `subscribe_runnable`:

- **Postgres** — trigger on `workflow` INSERT/UPDATE emits `pg_notify('assay_runnable', id)`;
  scheduler holds one `LISTEN assay_runnable` connection. Sub-ms delivery.
- **SQLite** — no cross-process push; returns an empty stream. Scheduler relies purely on its heap.
  Single-process deployments can still deliver in-process notifications via a channel.

The scheduler is never busy-polling. It sleeps until the heap's next timestamp OR a push
notification wakes it. Idle cost ≈ zero regardless of backend.

## Size, memory, and build cost

Estimates. Measure before publishing final numbers.

| Artifact / features                                               | Binary (stripped) | Cold build             |
| ----------------------------------------------------------------- | ----------------- | ---------------------- |
| `assay-lua` runtime (pg + sqlite, workflow, dashboard)            | ≤ 15 MB           | same as today + <30 s  |
| `assay-engine` binary, **default** (both backends: PG18 + SQLite) | ≤ 20 MB           | +2–3 min from pristine |
| `assay-engine` crate embedded in jeebon-api (default)             | +14–16 MB         | +2–3 min               |

Against a typical production stack `assay-engine` replaces:

| Service replaced         | Approx footprint    |
| ------------------------ | ------------------- |
| Keycloak / Zitadel (IdP) | 80–150 MB container |
| SpiceDB (Zanzibar)       | 40 MB Go daemon     |
| Temporal worker stack    | 100+ MB per pod     |
| **Total replaced**       | **~220–290 MB**     |

A ≤ 20 MB assay-engine binary is a net reduction of ~200 MB and two fewer services to operate.

## Versioning

Monorepo workspace, **independent crate versions** (tokio / serde / hyper precedent).

- Each crate has its own version field in its own `Cargo.toml`.
- `cargo-workspaces` (or `cargo-release`) drives per-crate publishing.
- Breaking change in one crate doesn't force bumps in unrelated crates.
- Shared traits in `assay-domain` stabilise first (`0.x` during early development); downstream
  crates re-export and rely on pinned ranges.

Consumer pinning works independently:

```toml
assay = "1.2" # stable runtime
assay-engine = "0.8" # faster-moving engine + auth
```

## Dashboard

Single crate `assay-dashboard`, two feature sets:

```toml
[features]
default = ["workflow"] # runtime binary uses this
full = ["workflow", "auth"] # engine binary uses this

workflow = [] # runs, events, timers, retries, archival views
auth = [] # client registry, users, sessions, Zanzibar tuple browser,
# JWKS rotation UI
```

Shared Askama templates, shared CSS, shared routing. Runtime builds only workflow views; engine
includes both. Future engine-only features (metrics, alerting) land behind additional feature flags
here.

## Migration phases

> **Superseded by plan 12.** The phase breakdown below is the original high-level sketch for the
> workflow-only scope. Plan 12 (and its sub-plans 12a–12e) is the authoritative execution plan
> covering workflow + auth + engine binary + CI for the v0.13.0 release. Consult plan 12 for current
> task ordering; the phases below remain useful as a conceptual overview.

### Phase 0 — scaffold crates (no behaviour change)

1. Create `crates/assay-domain` with shared types (move from `assay-workflow::types`).
2. Create empty `crates/assay-auth` and `crates/assay-engine`.
3. Extract current dashboard module from `assay-workflow` to `crates/assay-dashboard` (behind
   `workflow` feature).
4. Move top-level `src/` binary target to `crates/assay/`.

### Phase 1 — workflow storage as trait

5. Define `WorkflowStore` trait in `assay-domain`.
6. Move existing `postgres.rs` / `sqlite.rs` into feature-gated modules in
   `assay-workflow/src/store/`.
7. Re-wire scheduler + dispatcher to `&dyn WorkflowStore`.

### Phase 2 — engine binary

8. `bin/assay-engine.rs` with config file, backend selection, HTTP bind address.
9. Dashboard `full` feature; wire auth views stubs (filled by plan 11).

### Phase 3 — plan 11 auth lands on top

## AI-agent time estimate

| Phase                                                        | Hours  |
| ------------------------------------------------------------ | ------ |
| Phase 0 — scaffold crates, move types                        | 3      |
| Phase 1 — extract `WorkflowStore` trait, feature-gate impls  | 3      |
| Phase 2 — engine binary + dashboard feature-gating           | 6      |
| CI + release tooling (cargo-workspaces, independent publish) | 2      |
| Documentation (README, CHANGELOG, llms.txt)                  | 2      |
| **Total, before plan 11**                                    | **16** |

With two agents concurrently (Phase 1 + Phase 2), calendar ≈ 8 hours.

## Open decisions

he

1. **Runtime with no auth — accepted.** Lua scripts needing auth call engine over HTTP (0.5–2 ms
   localhost). Revisit if batch permission audits become common.

2. **Dashboard as one feature-gated crate — accepted.** Single source for templates and CSS.

3. **Independent crate versions.** Confirmed. Monorepo workspace, separate lifecycles.

4. **Hybrid dispatch wake-up from day one.** Scheduler owns a local timer heap; each backend
   supplies a push stream (`LISTEN/NOTIFY` for Postgres, empty for SQLite). No polling loop; no V2
   migration.

5. **PostgreSQL 18 is the minimum supported PostgreSQL version.** PG18 features leveraged include
   `uuidv7()` for time-ordered PKs, skip-scan composite indexes, and io_uring AIO. Earlier PG
   versions are not supported and not tested.

6. **Task visibility timeout + worker liveness.** Default visibility timeout 60 s (worker must
   heartbeat before it expires or the task is released). Dead-worker sweep every 30 s with cutoff 90
   s (i.e. missed 1.5 heartbeat intervals). Both configurable per namespace. Measure before locking
   defaults.

---

_Followed by: 11-engine-auth-modules.md._
