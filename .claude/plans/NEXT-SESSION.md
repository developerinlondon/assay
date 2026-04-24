# Pick up here — v0.13.0 SHIPPED; v0.13.1 (engine-events outbox) in flight; v0.14.0 (auth) queued

**Current branch:** `feature/0.13.1-engine-events-outbox` (tracks `origin/main`).

**Last shipped release:** v0.13.0 landed on `main` via PR #65 (merge commit `0164e8f`). Per-crate
tags `assay-v0.13.0`, `assay-engine-v0.1.0`, `assay-workflow-v0.2.0`, `assay-domain-v0.1.0`,
`assay-dashboard-v0.1.0`, `assay-auth-v0.1.0` shipped. Plan 12 phases 0-3, 9, 10 done.

## Release queue

| Release     | Scope                                                                                                                                                                      | Plan                                   | Status                                                                     |
| ----------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------- | -------------------------------------------------------------------------- |
| **v0.13.0** | engine-split + PG18/SQLite + pure `assay-lua` + engine binary + CI/release pipeline                                                                                        | plan 12 phases 0-3, 9, 10              | ✅ shipped on `main`                                                       |
| **v0.13.1** | Engine-events outbox — Rust-managed CDC, durable realtime SSE, backend-agnostic `EngineEventBus`, trigger removal, typed `WorkflowEventBus`, 15s scheduler-scan removal    | plan 13 (index + 13a–13g)              | 🚧 in flight on this branch                                                |
| **v0.14.0** | Auth — plan 12 phases 4-7: primitives (session/password/JWT/biscuit), identity flows (OIDC client/passkey), Zanzibar core on PG+SQLite, OIDC provider, compose into engine | plan 12c / 12d / 12e (reduced phase 8) | 🎯 queued; branch `feature/0.14.0-auth` from `main` once v0.13.1 tags land |

## v0.13.1 rationale (the detour before auth)

After v0.13.0 shipped, the PG `LISTEN/NOTIFY` + trigger path was assessed against Supabase Realtime
and SurrealDB LIVE SELECT. Two weak spots were flagged:

1. **SSE is at-most-once with no resumability.** Dashboards lose events when the laptop sleeps past
   the 1024-slot tokio::broadcast buffer.
2. **One PgListener connection per `subscribe_*` call.** Grows linearly with namespaces × nodes.

Plus a deeper architectural issue: **triggers are PG-only**, so the push-stream design is not
backend-agnostic. Required since production assay workflows will run on PG+SQLite, with multi-node
PG and jeebon integration on the roadmap.

Decision: build a Rust-managed CDC outbox (`engine_events`) that unifies the scheduler wake-up path,
task-worker wake-up, SSE streaming, and cross-node coordination under a single `EngineEventBus`
trait. PG uses one `NOTIFY` channel per namespace; SQLite is single-process with in-process
broadcast only. TCP keepalive + cursor replay replace polling. Triggers go away entirely.

Plan index: `.claude/plans/13-v0.13.1-engine-events-outbox.md` Phase files: `13a` (trait scaffold) →
`13b` (PG impl) → `13c` (SQLite impl) → `13d` (typed wrapper + cutover) → `13e` (delete triggers,
rewire scheduler) → `13f` (SSE rewrite) → `13g` (cleanup loop + keepalive + CHANGELOG + draft PR).

## Plan-12 phase status (authoritative)

| Phase                                                      | Scope         | Status            | Reference                 |
| ---------------------------------------------------------- | ------------- | ----------------- | ------------------------- |
| 0 — Scaffold 6 crates                                      | —             | ✅ v0.13.0        | plan 12a                  |
| 1 — State refactor (`WorkflowCtx<S>`, Arc-state, Shape 2B) | —             | ✅ v0.13.0        | plan 12a Task 1.3 revised |
| 2 — Parametrised harness                                   | —             | ✅ v0.13.0        | plan 12b Task 2.1         |
| ~~3 (old) — SurrealDB workflow backend~~                   | —             | 🗑️ removed (rev 2) | rev-2 decision log        |
| 3 (new) — Engine binary: workflow + dashboard              | —             | ✅ v0.13.0        | 12e + engine_smoke        |
| 4 — Auth primitives (session, password, jwt, biscuit)      | ~8h           | 🎯 v0.14.0        | plan 12c Phase 4          |
| 5 — Identity flows (OIDC client + passkey)                 | ~6h           | 🎯 v0.14.0        | plan 12c Phase 5          |
| 6 — Zanzibar core on PG + SQLite                           | ~8h           | 🎯 v0.14.0        | plan 12c Phase 6          |
| 7 — OIDC provider                                          | ~10h          | 🎯 v0.14.0        | plan 12d                  |
| 8 — Compose auth into engine                               | ~2h (reduced) | 🎯 v0.14.0        | 12e Phase 8               |
| 9 — CI per-crate + release workflow                        | —             | ✅ v0.13.0        | 12e Phase 9               |
| 10 — Docs + tag                                            | —             | ✅ v0.13.0        | 12e Phase 10              |

Interim plan 13 (v0.13.1 outbox) does not touch any plan-12 phase — additive, backend-agnostic, and
done on its own branch.

## What's next (in order)

### Right now — v0.13.1 execution

Walk through plan 13 phases 0-10 (phase files `13a`–`13g`). Phases 1-5 are additive; phase 6 is the
"no going back" deletion of triggers and `subscribe_*` methods. Phases 7-9 finish the SSE, cleanup,
and keepalive work. Phase 10 is CHANGELOG + per-crate bumps + draft PR. No migration guide — active
dev. Execution mode (subagent-driven vs inline) to be decided at kickoff.

Per-crate bumps for v0.13.1: `assay-v0.13.1`, `assay-engine-v0.1.1`, `assay-workflow-v0.2.1`,
`assay-domain-v0.1.1`. `assay-auth`, `assay-dashboard`, `assay-lua` unchanged (no content).

### Next release — v0.14.0 (auth)

After v0.13.1 tags land on `main`, cut `feature/0.14.0-auth` from `main`. Work through plan 12
phases 4–7 (auth primitives → identity flows → Zanzibar → OIDC provider), then the reduced phase 8
(wire auth into the engine). Plan files: `12c`, `12d`, `12e`.

Phase 4 kickoff task breakdown (from plan 12c):

- 4.1 — `assay-auth` Cargo feature matrix + store traits + `AuthCtx` skeleton + error enum (~1 h)
- 4.2 — Session module (cookies + CSRF + rotation) (~2 h)
- 4.3 — Password module (Argon2id) (~1 h)
- 4.4 — JWT module (issue + verify + JWKS rotation) (~2 h)
- 4.5 — Biscuit module (capability tokens) (~2 h)
- 4.6 — PG + SQLite User/Session store impls (~1 h)

Start with 4.1; subagent pattern recommended for impl-heavy 4.2–4.6. The v0.13.1 `EngineEventBus`
will already be in place, so Phase 4 can add an `AuthEventBus` typed wrapper for free.

## Critical context to carry forward (still valid for v0.14.0)

### Architecture decisions (don't re-litigate)

1. **`WorkflowCtx<S: WorkflowStore>` generic stays** — the trait has `impl Future` return types
   (RPITIT), not dyn-compatible. Static dispatch via monomorphisation; backend selection happens at
   engine `main()` startup via `match cfg.backend`. Plan 12 Architecture Principle 2.
2. **Shape 2B**: `WorkflowCtx<S>` is the axum state AND the orchestrator (no separate `Engine`
   struct). State type is `Arc<WorkflowCtx<S>>` per request. Plan 12a Task 1.3 revised.
3. **Layout 1 backends**: feature-gated inside each module crate (not separate
   `assay-workflow-postgres` crates). Plan 10.
4. **Runtime dashboard retired** — `assay-lua` runtime is scripts-only. `assay-engine` is the only
   path to a dashboard. Plan 12e Task 8.5 marked REMOVED.
5. **PG18 + SQLite only, both in one binary by default** (rev 2). SurrealDB dropped. Backends are
   additive features (not mutually exclusive); runtime picks one via `EngineConfig.backend`.
6. **v0.13.0 ships engine-only; v0.14.0 ships auth** (rev 3). Workflow API is open in v0.13.0
   (`AuthMode::no_auth()`); v0.14.0 flips to `AuthMode::jwt(self_issuer, audience)` pointing at the
   engine's own OIDC provider.
7. **`assay-lua` is a pure Lua runtime**, not an embedded engine. HTTP client talks to a deployed
   `assay-engine`. Plan 12 Principle 8.

### PG18 features we now rely on

`uuidv7()` for time-ordered PKs across all tables (workflows, events, tuples, users, sessions).
Skip-scan on composite indexes — one Zanzibar tuple index serves both forward and inverse queries.
`io_uring` AIO for polling scans on the workflow queue. Virtual generated columns for JSONB-derived
lookup keys. PG19's SQL/PGQ is a later-year additive upgrade; don't plan around it yet.

### Subagent usage lessons (still relevant for v0.14.0 batches)

Dispatch per-batch of 3–4 cohesive tasks, not per-task (too much coordination overhead) and not
whole-phase (too much risk of context exhaustion mid-task). Poll the subagent every 60 s proactively
— don't wait for the user to ask. When a subagent hangs on its own test-verification run, take over
directly. The Batch E push-stream subagent deadlocked trying to poll its own bg task last time;
lesson learned. Push-stream test lifecycle: if tests hang >60 s, the most likely cause is a spawned
poller task holding an `Arc<Harness>` forever, blocking testcontainer Drop — capture `JoinHandle`,
`.abort()` it at the end of the test.

### Cross-backend bug fixes we landed (keep for PG + SQLite)

SQLite's `delete_namespace` lacked the `AND name != 'main'` guard that PG had (pre-0.13 bug) — fixed
in Batch A. PG + SQLite `create_timer` had no `ON CONFLICT DO NOTHING` for `(workflow_id, seq)`
uniqueness — fixed in Batch B.

### Engine-smoke gotchas to remember

`cargo test --test engine_smoke` spawns the engine as a subprocess on an ephemeral port. If multiple
runs leak processes, `pkill -f assay-engine-preview` clears them. The test picks a free port via
`TcpListener::bind("127.0.0.1:0")`. Cron expressions in `assay-workflow` use the 6-field format (sec
min hour day month weekday), not 5-field Unix. API field is `cron_expr` not `cron`.

## Fast orientation for a fresh session

Open these in order:

1. `.claude/plans/12-v0.13.0-execution.md` — control doc + revision log + architecture principles
2. `.claude/plans/12c-phase-4-6-auth-identity-zanzibar.md` — phase 4 task list (next session's work
   starts here)
3. `.claude/plans/11-engine-auth-modules.md` — auth module rationale + technology choices (consumed
   by plan 12c for the why)
4. `crates/assay-auth/` — scaffolded crate, ready for Phase 4 content
5. `CHANGELOG.md` — v0.13.0 entry describes what shipped
6. `docs/migration-to-0.13.0.md` — for understanding the breaking API changes users face

## Git state

```
feature/0.13.0-engine-split
└── 1efebde fix(engine/seed): 6-field cron format + API field name
    (clean tree; release prep commits land next)

PR #65: open, draft — flipped to "ready" after CI green.
```

When v0.13.0 ships:

- Six per-crate tags pushed to `main`.
- v0.14.0 branches from `main` for the auth work (Phase 4 onward).
