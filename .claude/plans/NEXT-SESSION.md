# Pick up here — v0.13.0 engine split, Phase 4 next

**Branch:** `feature/0.13.0-engine-split` (do not merge to main yet — this is the 0.13.0 integration
branch).

**Last commit in the previous session:** `3a58c53` (docs: retire runtime dashboard permanently).

## Where we are

Phases done:

| Phase                                                            | Status | Ref                       |
| ---------------------------------------------------------------- | ------ | ------------------------- |
| 0 — Scaffold 6 crates                                            | ✅     | plan 12a                  |
| 1 — State refactor (`WorkflowCtx<S>`, Arc-state, Shape 2B merge) | ✅     | plan 12a Task 1.3 revised |
| 2 — Parametrised harness                                         | ✅     | plan 12b Task 2.1         |
| 3 — SurrealDB workflow backend (all 16 trait methods)            | ✅     | plan 12b Tasks 3.1–3.16   |

**Current test state**: `cargo test --workspace` → 1152 passing / 0 failing / 1 ignored (broadcast
doctest).

**Parametrised harness**:
`cargo test -p assay-workflow --test smoke_backends --features "backend-postgres backend-sqlite backend-surrealdb"`
→ 91 cases across PG/SQLite/Surreal all green.

## What's next — Phase 4

**Auth primitives.** Plan file: `.claude/plans/12c-phase-4-6-auth-identity-zanzibar.md`, tasks 4.1 –
4.6.

Six tasks:

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
   engine `main()` startup. Plan 12 Architecture Principle 2.
2. **Shape 2B**: `WorkflowCtx<S>` is the axum state AND the orchestrator (no separate `Engine`
   struct). State type is `Arc<WorkflowCtx<S>>` per request. Plan 12a Task 1.3 revised.
3. **Layout 1 backends**: feature-gated inside each module crate (not separate
   `assay-workflow-postgres` crates). Plan 10.
4. **Runtime dashboard retired** — `assay` runtime is scripts-only. `assay-engine` is the only path
   to a dashboard. Plan 12e Task 8.5 marked REMOVED.
5. **`assay-engine` defaults excluded `backend-surrealdb`** — surrealdb's transitive
   jsonwebtoken/aws-lc-rs feature conflicts with the runtime's rust_crypto when workspace feature
   unification kicks in. `cargo install assay-engine --features backend-surrealdb` is the opt-in
   path. Plan 12a Task 1.3 note.

### Subagent usage lessons

- **Dispatch per-batch of 3-4 cohesive tasks**, not per-task (too much coordination overhead) and
  not whole-phase (too much risk of context exhaustion mid-task).
- **Poll the subagent every 60s proactively** — don't wait for the user to ask. Long silences are
  usually "still compiling" or "testcontainer pulling image," but occasionally deadlocked polling
  loops.
- **When a subagent hangs on its own test-verification run**, take over directly. Don't let it
  grind. The Batch E push-stream subagent deadlocked trying to poll its own bg task — I finalized
  manually.
- **Push-stream test lifecycle**: if you see tests hang >60s, the most likely cause is a spawned
  poller task holding an `Arc<Harness>` forever, blocking testcontainer Drop. Fix: capture
  `JoinHandle`, `.abort()` it at the end of the test. Pattern in
  `push_runnable_fires_on_dispatchable` and friends.

### SurrealDB v3 quirks catalogue

Encountered during Phase 3:

- `type::thing` → `type::record` (renamed in v3).
- Record IDs with non-trivial keys serialise as `` `table:\`key\ `` ``.
  `SELECT record::id(id) AS id` strips table prefix; then strip the backticks too.
- `SCHEMAFULL` tables: every `CREATE` must populate all fields or silently drops records. Either
  populate or set `DEFINE FIELD ... DEFAULT ...` — use `DEFINE FIELD OVERWRITE` in new migrations
  rather than editing committed ones.
- `ORDER BY field` requires `field` in the `SELECT` list.
- Bind parameter names: avoid `$name`, `$id`, `$type` — they collide with SurrealDB parser patterns.
  Use `$sig_name`, `$wf_id` etc.
- `CREATE ... CONTENT` is SurrealDB's idempotency-friendly insert; use SELECT-then-CREATE for
  conditional idempotency.

### Cross-backend bug fixes we landed

- SQLite's `delete_namespace` lacked the `AND name != 'main'` guard that PG had (pre-0.13 bug).
  Fixed in Batch A.
- PG + SQLite `create_timer` had no `ON CONFLICT DO NOTHING` for `(workflow_id, seq)` uniqueness.
  Fixed in Batch B.

## Fast orientation for a fresh session

Open these in order:

1. `.claude/plans/12-v0.13.0-execution.md` — control doc + architecture principles
2. `.claude/plans/12c-phase-4-6-auth-identity-zanzibar.md` — phase 4 task list
3. `.claude/plans/11-engine-auth-modules.md` — auth module rationale + technology choices (consumed
   by plan 12c for the why)
4. `crates/assay-auth/` — currently scaffolded crate, stub features only

Then kick off Phase 4 Task 4.1. If inline is chosen, start with the Cargo.toml feature matrix + stub
`AuthCtx` struct. If subagent-driven, dispatch executor with the task text from plan 12c.

## Git state

```
feature/0.13.0-engine-split
└── 3a58c53 docs: retire runtime dashboard permanently
    (clean tree, nothing staged, nothing uncommitted)
```

No merge to main yet. Phase 10 Task 10.6 is the only point where main gets touched.
