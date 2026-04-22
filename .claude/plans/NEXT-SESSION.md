# Pick up here — v0.13.0 engine split (rev 2), strip SurrealDB → start Phase 4

**Branch:** `feature/0.13.0-engine-split` (do not merge to main yet — this is the 0.13.0 integration
branch).

**Last commit in the previous session:** `9b585d4` (docs: NEXT-SESSION pickup note for resuming
v0.13.0 work).

## v0.13.0 rev 2 decision (2026-04-22) — read first

**SurrealDB is removed from v0.13.0.** Backends are PostgreSQL 18 (default) + SQLite. Both compile
into `assay-engine` by default; the active backend is runtime-selected from `EngineConfig.backend`.
Full rationale in `.claude/plans/12-v0.13.0-execution.md` "Revision log" section. Summary:

- SurrealDB tripled the clean-release build time (91 s → 281 s) and nearly tripled peak compile RAM
  (1.3 GB → 3.7 GB) on this workspace; binary size delta was negligible.
- The prior "jsonwebtoken + aws-lc-rs conflicts with rust_crypto" rationale was wrong — `cargo tree`
  proves those crates are present in the baseline PG+SQLite build via sqlx + reqwest.
- PostgreSQL 18 + `pgvector` + recursive CTEs covers the Zanzibar tuple store, family-tree / folder
  traversal, and vector search with no lost capability.

## Where we are

Phases done before rev 2:

| Phase                                                            | Status | Ref                       |
| ---------------------------------------------------------------- | ------ | ------------------------- |
| 0 — Scaffold 6 crates                                            | ✅     | plan 12a                  |
| 1 — State refactor (`WorkflowCtx<S>`, Arc-state, Shape 2B merge) | ✅     | plan 12a Task 1.3 revised |
| 2 — Parametrised harness                                         | ✅     | plan 12b Task 2.1         |
| 3 — SurrealDB workflow backend                                   | 🗑️      | **REMOVED per rev 2**     |

Rev-2 strip is a NEW pre-Phase-4 task in `.claude/plans/12-v0.13.0-execution.md`. Must happen before
Phase 4 starts. Files to delete / edit:

- `crates/assay-workflow/src/store/surrealdb.rs`
- `backend-surrealdb` feature in `assay-workflow/Cargo.toml`, `assay-auth/Cargo.toml`,
  `assay-engine/Cargo.toml`, `assay-lua (crates/assay)/Cargo.toml`
- `surrealdb`, `surrealdb-*` dependencies at the workspace level
- Testcontainer image pulls for SurrealDB
- `smoke_backends` test — drop the Surreal parameter
- `crates/assay-workflow/migrations/surrealdb/` directory

**Expected test state after strip**: `cargo test --workspace` should stay green (≈ 1152 − Surreal
cases = ~1090 tests green) because SurrealDB's impl did not touch the trait or the PG/SQLite impls.

## What's next — rev-2 strip, then Phase 4

### Step 0 — rev-2 SurrealDB strip (~1.5 h)

Mechanical. One commit: `refactor: drop SurrealDB backend per plan 12 rev 2`. After commit, re-run
`cargo test --workspace` and confirm green.

### Phase 4 — auth primitives (~8 h)

Plan: `.claude/plans/12c-phase-4-6-auth-identity-zanzibar.md` tasks 4.1 – 4.6.

- 4.1 — `assay-auth` Cargo feature matrix + store traits + `AuthCtx` skeleton + error enum (~1h)
- 4.2 — Session (cookies + CSRF + rotation) (~2h)
- 4.3 — Password (Argon2id) (~1h)
- 4.4 — JWT (issue + verify + JWKS rotation) (~2h)
- 4.5 — Biscuit (capability tokens) (~2h)
- 4.6 — PG + SQLite User/Session store impls (~1h)

Start with 4.1 — it's the foundation. Dispatch a subagent or do inline; either works. Subagent
pattern (used for Phase 3 batches) is recommended for the impl-heavy tasks 4.2-4.6 — they're
mechanical mirrors of the spec in plan 12c + plan 11.

## Critical context to carry forward

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
5. **Rev 2: PG18 + SQLite only, both in one binary by default.** SurrealDB dropped. Backends are
   additive features (not mutually exclusive); runtime picks one via `EngineConfig.backend`. Users
   can build with `--no-default-features --features backend-sqlite` (or backend-postgres) for a
   smaller binary if they're single-backend. See plan 12 rev-2 decision log.

### PG18 features we now rely on

- `uuidv7()` for time-ordered PKs across all tables (workflows, events, tuples, users, sessions).
- Skip-scan on composite indexes (e.g. one Zanzibar tuple index serves both forward and inverse
  queries).
- `io_uring` AIO for polling scans on the workflow queue (`SELECT … FOR UPDATE SKIP LOCKED`).
- Virtual generated columns for JSONB-derived lookup keys.
- PG19's SQL/PGQ is a later-year additive upgrade; don't plan around it yet.

### Subagent usage lessons

- **Dispatch per-batch of 3-4 cohesive tasks**, not per-task (too much coordination overhead) and
  not whole-phase (too much risk of context exhaustion mid-task).
- **Poll the subagent every 60s proactively** — don't wait for the user to ask.
- **When a subagent hangs on its own test-verification run**, take over directly. Don't let it
  grind. The Batch E push-stream subagent deadlocked trying to poll its own bg task — I finalized
  manually.
- **Push-stream test lifecycle**: if you see tests hang >60s, the most likely cause is a spawned
  poller task holding an `Arc<Harness>` forever, blocking testcontainer Drop. Fix: capture
  `JoinHandle`, `.abort()` it at the end of the test. Pattern in
  `push_runnable_fires_on_dispatchable` and friends.

### Cross-backend bug fixes we landed (keep for PG + SQLite)

- SQLite's `delete_namespace` lacked the `AND name != 'main'` guard that PG had (pre-0.13 bug).
  Fixed in Batch A.
- PG + SQLite `create_timer` had no `ON CONFLICT DO NOTHING` for `(workflow_id, seq)` uniqueness.
  Fixed in Batch B.

### Deleted with rev 2 (for history — don't re-introduce)

- SurrealDB v3 quirks catalogue (`type::thing` → `type::record`, bind-parameter name collisions,
  `SCHEMAFULL` field rules, `record::id()` backtick stripping). No longer relevant.
- Testcontainer lifecycle notes for SurrealDB stream-pollers.

## Fast orientation for a fresh session

Open these in order:

1. `.claude/plans/12-v0.13.0-execution.md` — control doc + revision log + architecture principles
2. `.claude/plans/12c-phase-4-6-auth-identity-zanzibar.md` — phase 4 task list
3. `.claude/plans/11-engine-auth-modules.md` — auth module rationale + technology choices (consumed
   by plan 12c for the why)
4. `crates/assay-auth/` — currently scaffolded crate, stub features only

Then: execute the rev-2 strip (Step 0 above), then kick off Phase 4 Task 4.1.

## Git state

```
feature/0.13.0-engine-split
└── 9b585d4 docs: NEXT-SESSION pickup note for resuming v0.13.0 work
    (clean tree; plan revisions in flight for rev 2)
```

No merge to main yet. Phase 10 Task 10.6 is the only point where main gets touched.
