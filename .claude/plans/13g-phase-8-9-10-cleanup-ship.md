# Plan 13g — Phase 8 + 9 + 10: Cleanup Loop, TCP Keepalive, Ship

> Parent plan: [13-v0.13.1-engine-events-outbox.md](13-v0.13.1-engine-events-outbox.md) Prev:
> [13f-phase-7-sse-rewrite.md](13f-phase-7-sse-rewrite.md) — **Final phase**

---

## Phase 8 — Cleanup loop + retention config

**Files:**

- Modify: `crates/assay-engine/src/config.rs` (or wherever `EngineConfig` lives)
- Create: `crates/assay-workflow/src/events_cleanup.rs`
- Modify: `crates/assay-workflow/src/lib.rs`
- Modify: `crates/assay-workflow/src/ctx.rs` (spawn cleanup loop in `start`)
- Create: `crates/assay-workflow/tests/engine_events_cleanup.rs`

- [ ] **Step 8.1: Add `engine_events_ttl_secs` to `EngineConfig`**

Find where `EngineConfig` is defined (grep `pub struct EngineConfig` under
`crates/assay-engine/src/`). Add:

```rust
pub struct EngineConfig {
    // ... existing fields
    /// TTL in seconds for the engine_events outbox. Rows older than
    /// this are pruned hourly by the cleanup loop. Default 3 days.
    pub engine_events_ttl_secs: u64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            // ... existing defaults
            engine_events_ttl_secs: 3 * 86_400,
        }
    }
}
```

Also surface the field through whatever TOML/env parsing the config layer uses. If the config is
serde-derived, the default attribute covers it:

```rust
#[serde(default = "default_engine_events_ttl_secs")]
pub engine_events_ttl_secs: u64,

fn default_engine_events_ttl_secs() -> u64 { 3 * 86_400 }
```

- [ ] **Step 8.2: Create the cleanup loop**

Create `crates/assay-workflow/src/events_cleanup.rs`:

```rust
//! Hourly prune of `engine_events` older than the configured TTL.
//! Idempotent across nodes — `DELETE WHERE ts < cutoff` is a no-op
//! if another node already swept. No leader election needed.

use std::sync::Arc;
use std::time::Duration;

use assay_domain::events::EngineEventBus;

pub async fn run_events_cleanup(
    bus: Arc<dyn EngineEventBus>,
    cadence: Duration,
    ttl_secs: u64,
) {
    let mut tick = tokio::time::interval(cadence);
    // First tick fires immediately; skip it so we don't prune at startup.
    tick.tick().await;
    loop {
        tick.tick().await;
        let cutoff = now_secs() - ttl_secs as f64;
        match bus.prune(cutoff).await {
            Ok(n) if n > 0 => tracing::info!(pruned = n, "engine_events cleanup swept"),
            Ok(_) => tracing::debug!("engine_events cleanup: nothing to prune"),
            Err(e) => tracing::warn!(?e, "engine_events prune failed; will retry next tick"),
        }
    }
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
```

Add to `crates/assay-workflow/src/lib.rs`:

```rust
pub mod events_cleanup;
```

- [ ] **Step 8.3: Spawn the cleanup loop at engine startup**

In `crates/assay-workflow/src/api/mod.rs::serve` (right after constructing the bus), spawn the
cleanup task:

```rust
let _cleanup_handle = tokio::spawn(
    crate::events_cleanup::run_events_cleanup(
        Arc::clone(&bus),
        Duration::from_secs(3600),
        cfg.engine_events_ttl_secs,
    )
);
```

Since `cfg` isn't currently passed into `serve`, add a parameter or wrap into a new
`serve_with_config`. Preferred: extend the `serve` signature with `engine_events_ttl_secs: u64` so
it's minimal at this layer, and the binary in `assay-engine` passes `cfg.engine_events_ttl_secs` in.

- [ ] **Step 8.4: Write the cleanup test**

Create `crates/assay-workflow/tests/engine_events_cleanup.rs`:

```rust
#![cfg(feature = "backend-sqlite")]

use std::sync::Arc;

use assay_domain::events::{EngineEventBus, NewEvent, SqliteEngineEventBus, Subsystem};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

async fn fresh_bus() -> Arc<dyn EngineEventBus> {
    let opts = SqliteConnectOptions::new()
        .filename(":memory:")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS engine_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts REAL NOT NULL DEFAULT (CAST(strftime('%s','now') AS REAL)),
            namespace TEXT NOT NULL,
            subsystem TEXT NOT NULL,
            kind TEXT NOT NULL,
            payload TEXT NOT NULL DEFAULT '{}')",
    )
    .execute(&pool)
    .await
    .unwrap();
    Arc::new(SqliteEngineEventBus::new(pool).await.unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn cleanup_prunes_old_events_and_is_idempotent() {
    let bus = fresh_bus().await;
    for _ in 0..5 {
        bus.publish_committed(NewEvent {
            namespace: "main",
            subsystem: Subsystem::Workflow,
            kind: "x",
            payload: serde_json::json!({}),
        })
        .await
        .unwrap();
    }
    let n1 = bus.prune(f64::MAX).await.unwrap();
    assert_eq!(n1, 5);
    let n2 = bus.prune(f64::MAX).await.unwrap();
    assert_eq!(n2, 0, "prune must be idempotent");
}
```

- [ ] **Step 8.5: Verify + commit**

```bash
cargo test -p assay-workflow --features backend-sqlite --test engine_events_cleanup
```

Expected: PASS. Commit:

```bash
git add crates/
git commit -m "$(cat <<'EOF'
feat(workflow): engine_events cleanup loop + configurable TTL

- engine_events_ttl_secs: u64 on EngineConfig, default 3 * 86400.
- tokio::spawn on a 1-hour interval in api::serve; idempotent
  DELETE WHERE ts < cutoff. No leader election — running twice is a
  no-op. First tick skipped so we don't prune at startup.
- Cleanup test asserts idempotency (second prune returns 0).
EOF
)"
```

---

## Phase 9 — TCP keepalive on the PgListener connection

Phase 2 already adds `listener_opts` with keepalives enabled. This phase verifies the wiring with a
smoke test and confirms the config reaches production.

**Files:**

- Possibly modify: `crates/assay-domain/src/events/pg.rs` (already done in Phase 2; verify)
- Possibly modify: `crates/assay-engine/src/bin/assay-engine.rs` (ensure the DB URL is threaded
  through)

- [ ] **Step 9.1: Verify the `PgEngineEventBus::new` signature takes `db_url`**

From Phase 2:

```rust
pub async fn new(pool: PgPool, db_url: &str) -> Result<Self> {
    let listener_opts = PgConnectOptions::from_str(db_url)?
        .keepalives(true)
        .keepalives_idle(Duration::from_secs(30))
        .keepalives_interval(Duration::from_secs(10))
        .keepalives_retries(3);
    // ...
}
```

Confirm this is in place (`grep -n keepalives crates/assay-domain/src/events/pg.rs`).

- [ ] **Step 9.2: Confirm the binary passes the URL in**

In `crates/assay-engine/src/bin/assay-engine.rs`, the PG construction branch should read:

```rust
Backend::Postgres => {
    let pool = PgPool::connect(&cfg.db_url).await?;
    Arc::new(
        assay_domain::events::PgEngineEventBus::new(pool, &cfg.db_url).await?,
    )
}
```

If it currently uses a different pool-construction path (e.g. `PostgresStore::new(&url)` that hides
the URL), expose the URL at the config level: `cfg.db_url: String`.

- [ ] **Step 9.3: Optional smoke test for listener reconnection**

Add to `crates/assay-domain/src/events/pg_test.rs`:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn listener_auto_reconnects_after_disconnect() {
    // Simulate a PG client disconnect by spawning a bus, publishing
    // once, dropping the bus, then constructing a new one against the
    // same pool and confirming it still receives publishes. sqlx's
    // auto-reconnect covers the "pool-level" reconnect; the harder
    // case (server-side termination) requires pg_terminate_backend on
    // the listener's PID which is flaky in CI — keep this test
    // qualitative.
    //
    // Minimum assertion: creating two Bus instances against the same
    // pool works and both can subscribe and receive.

    let pool = fresh_pool().await;
    let url = std::env::var("ASSAY_PG_TEST_URL").unwrap();
    let bus1 = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
    drop(bus1);
    let bus2 = PgEngineEventBus::new(pool.clone(), &url).await.unwrap();
    let mut rx = bus2.subscribe("main");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    bus2.publish_committed(NewEvent {
        namespace: "main",
        subsystem: Subsystem::Workflow,
        kind: "z",
        payload: serde_json::json!({}),
    })
    .await
    .unwrap();
    let ev = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ev.kind, "z");
}
```

- [ ] **Step 9.4: Verify + commit**

```bash
cargo test -p assay-domain --features backend-postgres --lib -- pg_test
```

Expected: all 7 tests PASS (6 from Phase 2 + the new reconnect smoke).

```bash
git add crates/assay-domain/src/events/pg_test.rs
git commit -m "$(cat <<'EOF'
test(domain/events/pg): smoke test for listener reconnect across bus drop

Adds a lightweight assertion that two PgEngineEventBus instances
constructed against the same pool each get a working LISTEN bridge
(exercised by publish+subscribe across a bus drop). Full server-side
backend-termination test is omitted (flaky in CI).

TCP keepalive (30s/10s/3) is already set up in Phase 2;
this test documents the reconnect expectation.
EOF
)"
```

---

## Phase 10 — CHANGELOG + per-crate bumps + draft PR

Active development; no migration guide per decision #14.

**Files:**

- Modify: `CHANGELOG.md`
- Modify: `crates/{assay,assay-engine,assay-workflow,assay-domain}/Cargo.toml` (version bumps)

- [ ] **Step 10.1: Add CHANGELOG entry**

In `CHANGELOG.md` above the v0.13.0 section:

```markdown
## v0.13.1 (2026-04-XX)

### Active-development notes

This is an active-development release; no migration guide — consumers roll with the engine at each
bump.

### Added

- `assay-domain::events::EngineEventBus` trait + `PgEngineEventBus` + `SqliteEngineEventBus`
  implementations.
- `engine_events` table (PG + SQLite) as the durable event outbox.
- `WorkflowEventBus` + `WorkflowEvent` enum in `assay-workflow`.
- `EngineConfig.engine_events_ttl_secs` (default `259200` = 3 days).
- Hourly cleanup task for `engine_events`.
- SSE `/api/v1/events/stream` now supports `Last-Event-ID` replay, HTTP 410 Gone on pre-retention
  cursor, and `?ns=&subsystem=&workflow_id=&kind=` filters.

### Changed

- SSE payload shape expanded: `{id, ts, namespace, subsystem, kind, payload}`. Old shape
  (`{event_type, workflow_id, payload}`) is a subset of the new payload's contents, so dashboards
  keep working if they only read the new fields they need.
- Scheduler wake-up is now cross-node capable without per-subscription PgListener connections.
- `dispatch_recovery` cadence bumped from 15s to 10min (pure hygiene net; durable outbox replay is
  the correctness source).
- PgListener uses explicit TCP keepalives (30s idle / 10s interval / 3 retries).

### Removed

- PL/pgSQL triggers `assay_notify_runnable`, `assay_notify_task`.
- `assay_runnable_<ns>` / `assay_task_<queue>` NOTIFY channels (replaced by one
  `assay_engine_events_<ns>` channel per namespace).
- `WorkflowStore::subscribe_runnable` and `subscribe_tasks` trait methods.
- In-memory `sse_tx` / `engine_tx` broadcast channels on `WorkflowCtx`.
- 15s periodic scheduler scan loop.

### Fixed

- Dashboard SSE clients no longer lose events when the laptop sleeps for longer than the broadcast
  buffer. Cursor-based replay refills the gap up to the retention window.
```

- [ ] **Step 10.2: Per-crate version bumps**

Edit each Cargo.toml:

| File                               | From                 | To                   |
| ---------------------------------- | -------------------- | -------------------- |
| `crates/assay/Cargo.toml`          | `version = "0.13.0"` | `version = "0.13.1"` |
| `crates/assay-engine/Cargo.toml`   | `version = "0.1.0"`  | `version = "0.1.1"`  |
| `crates/assay-workflow/Cargo.toml` | `version = "0.2.0"`  | `version = "0.2.1"`  |
| `crates/assay-domain/Cargo.toml`   | `version = "0.1.0"`  | `version = "0.1.1"`  |

Leave `assay-dashboard`, `assay-auth`, `assay-lua` untouched — no content changed.

- [ ] **Step 10.3: Final full-workspace check**

```bash
cargo check --workspace --all-features 2>&1 | tail -5
cargo test --workspace --lib --tests 2>&1 | tail -20
cargo test --test engine_smoke 2>&1 | tail -10
```

Expected: all green.

- [ ] **Step 10.4: Commit bumps + CHANGELOG**

```bash
git add CHANGELOG.md crates/*/Cargo.toml
git commit -m "$(cat <<'EOF'
chore(release): v0.13.1 — engine-events outbox

Per-crate bumps:
  assay          0.13.0 → 0.13.1
  assay-engine   0.1.0  → 0.1.1
  assay-workflow 0.2.0  → 0.2.1
  assay-domain   0.1.0  → 0.1.1

CHANGELOG entry captures added / changed / removed surfaces.
Active-development release; no migration guide.
EOF
)"
```

- [ ] **Step 10.5: Open draft PR**

```bash
git push -u origin feature/0.13.1-engine-events-outbox
gh pr create --title "v0.13.1 — engine-events outbox (durable realtime, backend-agnostic)" --body "$(cat <<'EOF'
## Summary

- Rust-managed CDC outbox (`engine_events`) replaces the PL/pgSQL
  triggers and the lossy in-memory SSE broadcast. All state-mutating
  workflow methods emit typed `WorkflowEvent` variants via
  `EngineEventBus`.
- Durable realtime: browser SSE clients reconnect with `Last-Event-ID`
  and replay up to 3 days of missed events. Pre-retention → HTTP 410.
- Backend-agnostic: PG and SQLite implement the same trait. PG adds
  cross-node wake-up via one `NOTIFY` channel per namespace.
- Zero polling: TCP keepalive on the `PgListener` + sqlx auto-reconnect
  + cursor replay covers all liveness gaps. The 15s periodic scheduler
  scan is removed.

See `.claude/plans/13-v0.13.1-engine-events-outbox.md` (index) and
`13a`–`13g` for the full rationale + phase-by-phase plan.

## Test plan

- [x] Per-backend unit tests for `EngineEventBus` (append/read/subscribe/prune/gone/reconnect)
- [x] Cross-node NOTIFY propagation test (PG)
- [x] SSE integration: replay from `Last-Event-ID`, 410 on gap
- [x] Cleanup loop idempotency test
- [x] `cargo test --workspace --lib --tests`
- [x] `cargo test --test engine_smoke`
- [x] `cargo check --workspace --all-features`
EOF
)" --draft
```

Flip from draft to ready once CI is green.

---

## Exit criteria for v0.13.1

```bash
cargo check --workspace --all-features      # clean
cargo test --workspace --lib --tests        # all green
cargo test --test engine_smoke              # green
git log --oneline main..HEAD                # phases 0-10 commits on branch
gh pr view                                  # PR exists
```

After merge: the main branch tags `assay-v0.13.1`, `assay-engine-v0.1.1`, `assay-workflow-v0.2.1`,
`assay-domain-v0.1.1` drive the existing release workflow.

**Next release on the queue:** plan 12 phases 4–7 (auth primitives, identity flows, Zanzibar, OIDC
provider) ship as v0.14.0 from a fresh `feature/0.14.0-auth` branch.
