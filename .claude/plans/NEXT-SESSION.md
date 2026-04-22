# Pick up here — v0.13.0 shipped; v0.14.0 starts with Phase 4 (auth)

**Branch:** `feature/0.13.0-engine-split` at the time of writing. Once v0.13.0 tags land on `main`,
start `feature/0.14.0-auth` from `main` for the next release cycle.

**Last commit on this branch (v0.13.0 readiness):** `1efebde` (fix(engine/seed): 6-field cron
format + API field name).

## v0.13.0 rev 3 decision (2026-04-22) — read first

The original plan 12 goal bundled workflow + auth in a single v0.13.0 release. After the engine
binary (Phase 3) wired up cleanly on the branch, the decision was made to ship engine-split +
workflow as **v0.13.0** on its own and defer auth to **v0.14.0**. Reasons: the engine is already
demoable and usable for jeebon workflow integration; holding it back behind Phases 4–7 (~50 more
hours of scope) blocks real integration feedback; smaller release chunks mean smaller upgrade pain
for consumers. Full context in plan 12's rev-3 revision log.

**v0.13.0 ships**: engine-split (6 crates), PG18 + SQLite backends, SurrealDB dropped entirely,
`assay-lua` reduced to a pure Lua runtime + HTTP client, `assay-engine` binary runnable with
workflow + dashboard (no auth). **v0.14.0 will ship** auth primitives (Phase 4), OIDC client +
passkey (Phase 5), Zanzibar core on PG + SQLite (Phase 6), full OIDC provider (Phase 7), and
composition into the engine (Phase 8 — now small: just wire the auth modules that Phases 4–7 built).

## Where we are

| Phase                                                      | Status     | Ref                         |
| ---------------------------------------------------------- | ---------- | --------------------------- |
| 0 — Scaffold 6 crates                                      | ✅ v0.13.0 | plan 12a                    |
| 1 — State refactor (`WorkflowCtx<S>`, Arc-state, Shape 2B) | ✅ v0.13.0 | plan 12a Task 1.3 revised   |
| 2 — Parametrised harness                                   | ✅ v0.13.0 | plan 12b Task 2.1           |
| ~~3 — SurrealDB workflow backend~~                         | 🗑️ removed  | rev-2 decision log          |
| 3 — Engine binary: workflow + dashboard (new Phase 3)      | ✅ v0.13.0 | 12e-derived + engine_smoke  |
| 4 — Auth primitives: session, password, jwt, biscuit       | 🎯 v0.14.0 | plan 12c Phase 4            |
| 5 — Identity flows: OIDC client + passkey                  | 🎯 v0.14.0 | plan 12c Phase 5            |
| 6 — Zanzibar core: PG + SQLite                             | 🎯 v0.14.0 | plan 12c Phase 6            |
| 7 — OIDC provider                                          | 🎯 v0.14.0 | plan 12d                    |
| 8 — Compose auth into engine                               | 🎯 v0.14.0 | 12e Phase 8 (reduced scope) |
| 9 — CI per-crate + release workflow                        | 🎯 v0.13.0 | 12e Phase 9 (in this PR)    |
| 10 — Docs + migration + tag                                | 🎯 v0.13.0 | 12e Phase 10 (in this PR)   |

Phases 9–10 are finishing on this branch as part of the v0.13.0 release prep. After the v0.13.0 tag
lands on `main`, cut `feature/0.14.0-auth` for the auth work.

## What's next

### This branch (v0.13.0 ship tasks, in flight)

1. **CI + release workflow updates** — `.github/workflows/ci.yml` gets a PG18 service container +
   engine_smoke integration test; `.github/workflows/release.yml` switches to per-crate tag pattern
   (`*-v*`) + crates.io publish step. See PR #65 for progress.
2. **`cargo publish --dry-run` per crate** — catch publish blockers (license files, repo URL,
   description, etc.).
3. **Merge PR #65** after CI is green.
4. **Tag** six per-crate tags: `assay-v0.13.0`, `assay-engine-v0.1.0`, `assay-workflow-v0.2.0`,
   `assay-core-v0.1.0`, `assay-dashboard-v0.1.0`, `assay-auth-v0.1.0`. CI publish-on-tag does the
   crates.io upload + GitHub release artefacts per existing `release.yml` pattern.

### Next session (v0.14.0, new branch)

Start `feature/0.14.0-auth` from `main` after v0.13.0 ships. Phase 4 — auth primitives (~8 h):

- 4.1 — `assay-auth` Cargo feature matrix + store traits + `AuthCtx` skeleton + error enum (~1 h)
- 4.2 — Session module (cookies + CSRF + rotation) (~2 h)
- 4.3 — Password module (Argon2id) (~1 h)
- 4.4 — JWT module (issue + verify + JWKS rotation) (~2 h)
- 4.5 — Biscuit module (capability tokens) (~2 h)
- 4.6 — PG + SQLite User/Session store impls (~1 h)

Start with 4.1 — it's the foundation. Dispatch a subagent or do inline; either works. Subagent
pattern is recommended for the impl-heavy tasks 4.2–4.6.

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
