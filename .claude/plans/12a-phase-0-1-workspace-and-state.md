# 12a — Phases 0 + 1 — Workspace scaffold and state refactor

> Sub-plan of [12-v0.13.0-execution.md](./12-v0.13.0-execution.md). Execute with
> superpowers:subagent-driven-development or superpowers:executing-plans.

**Phase 0 goal:** All six crates exist, the root binary lives at `crates/assay/`, workspace builds
clean, existing `assay` behaviour is unchanged.

**Phase 1 goal:** `Engine<S>` generic parameter is gone; handlers use per-module `Ctx` types;
`FromRef` is wired so the forthcoming `assay-engine` composition is just `.merge(...)` calls. Still
single-crate by backend at this point — nothing has moved yet.

**Prior work already on this branch** (commits `5bbded2`, `71a8abb`): `crates/assay-core` and
`crates/assay-auth` scaffolds are live. Tasks 0.1 and 0.2 from the earlier plan draft are done;
skip.

---

## Phase 0 — Workspace scaffold

### Task 0.3: Create `assay-dashboard` crate (typed asset bundle)

**Files:**

- Create: `crates/assay-dashboard/Cargo.toml`
- Create: `crates/assay-dashboard/src/lib.rs`
- Modify: root `Cargo.toml` (add to workspace members)

Scope note: this task scaffolds the crate and moves NO code. The actual static-asset relocation +
router extraction happens in Task 1.6, after the state refactor has established the `DashboardCtx`
shape.

- [ ] **Step 1: Create Cargo.toml**

```toml
# crates/assay-dashboard/Cargo.toml
[package]
name = "assay-dashboard"
version = "0.1.0"
edition = "2024"
license = "Apache-2.0"
repository = "https://github.com/developerinlondon/assay"
description = "Dashboard (HTML + JS + CSS) for assay-workflow and assay-engine. Feature-gated per view set."
categories = ["web-programming::http-server"]
keywords = ["dashboard", "workflow", "auth", "assay"]

[features]
default = ["workflow"]
workflow = []
auth = []

[dependencies]
assay-core = { path = "../assay-core", version = "0.1" }
axum = "0.8"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

- [ ] **Step 2: Stub lib.rs**

```rust
//! Dashboard — typed asset bundle + axum router composition.
//!
//! Feature flags:
//!  - `workflow` (default): workflow run lists, events, timers, retries
//!  - `auth`: user + session + Zanzibar + OIDC client registry views
//!
//! The real asset relocation lands in plan 12a Task 1.6 alongside the
//! `DashboardCtx` state refactor. This file is a scaffold until then.
```

- [ ] **Step 3: Add to workspace**

Edit `/Cargo.toml` — add `"crates/assay-dashboard"` to `[workspace] members`.

- [ ] **Step 4: Verify**

```bash
cargo check --workspace
```

Expected: `Checking assay-dashboard v0.1.0` appears; no errors.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/assay-dashboard
git commit -m "feat(dashboard): scaffold assay-dashboard crate"
```

---

### Task 0.4: Create `assay-engine` crate + binary stub

**Files:**

- Create: `crates/assay-engine/Cargo.toml`
- Create: `crates/assay-engine/src/lib.rs`
- Create: `crates/assay-engine/src/bin/assay-engine.rs`
- Modify: root `Cargo.toml`

- [ ] **Step 1: Cargo.toml**

```toml
[package]
name = "assay-engine"
version = "0.1.0"
edition = "2024"
license = "Apache-2.0"
repository = "https://github.com/developerinlondon/assay"
description = "Standalone workflow + auth + dashboard HTTP server. Embeddable as a library, or run as a binary with pluggable Postgres / SQLite / SurrealDB backends."
categories = ["asynchronous", "database", "web-programming::http-server"]
keywords = ["workflow", "auth", "oidc", "zanzibar", "surrealdb"]

[[bin]]
name = "assay-engine"
path = "src/bin/assay-engine.rs"
required-features = ["server"]

[lib]
name = "assay_engine"
path = "src/lib.rs"

[features]
default = [
  "workflow",
  "auth",
  "dashboard",
  "backend-postgres",
  "backend-sqlite",
  "backend-surrealdb",
  "server",
]

workflow = ["dep:assay-workflow", "assay-dashboard/workflow"]
auth = ["dep:assay-auth", "assay-dashboard/auth"]
dashboard = ["dep:assay-dashboard"]
server = ["dep:axum", "dep:tokio", "dep:tower-http", "dep:clap"]

backend-postgres = [
  "assay-workflow?/backend-postgres",
  "assay-auth?/backend-postgres",
]
backend-sqlite = [
  "assay-workflow?/backend-sqlite",
  "assay-auth?/backend-sqlite",
]
backend-surrealdb = [
  "assay-workflow?/backend-surrealdb",
  "assay-auth?/backend-surrealdb",
]

[dependencies]
assay-core = { path = "../assay-core", version = "0.1" }
assay-workflow = { path = "../assay-workflow", version = "0.2", optional = true }
assay-auth = { path = "../assay-auth", version = "0.1", optional = true }
assay-dashboard = { path = "../assay-dashboard", version = "0.1", optional = true }

axum = { version = "0.8", optional = true }
tokio = { version = "1", features = ["full"], optional = true }
tower-http = { version = "0.6", features = ["cors", "trace"], optional = true }
clap = { version = "4", features = ["derive", "env"], optional = true }

serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.9"
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
```

Note the `?` syntax on feature deps — `assay-workflow?/backend-postgres` means "if assay-workflow
feature is enabled, also enable its backend-postgres feature." This keeps the `assay-core`-only
build path working.

- [ ] **Step 2: Stub lib.rs**

```rust
//! Assay engine — workflow + auth + dashboard as a crate or standalone binary.
//!
//! State is composed via axum's `FromRef` — each module supplies its own
//! `Ctx` type and router; `EngineState` bundles them. See plan 12 §
//! Architecture principle 1.

#[cfg(feature = "workflow")]
pub use assay_workflow as workflow;

#[cfg(feature = "auth")]
pub use assay_auth as auth;

#[cfg(feature = "dashboard")]
pub use assay_dashboard as dashboard;

pub use assay_core as core;

pub mod config;
pub mod state;
pub mod server;
```

Create stub modules:

```rust
// crates/assay-engine/src/config.rs
//! Engine configuration (TOML). Full schema lands in Phase 8.
```

```rust
// crates/assay-engine/src/state.rs
//! EngineState — composes module Ctx types via FromRef. Full definition
//! lands in Phase 8 once all module Ctx types are in place.
```

```rust
// crates/assay-engine/src/server.rs
//! HTTP server wiring. Full router composition lands in Phase 8.
```

- [ ] **Step 3: Stub binary**

```rust
// crates/assay-engine/src/bin/assay-engine.rs
//! Standalone assay-engine binary. Real wiring lands in plan 12e Phase 8.

fn main() {
    eprintln!(
        "assay-engine {} — binary wiring deferred to phase 8 (see plan 12)",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(2);
}
```

- [ ] **Step 4: Register workspace member**

Add `"crates/assay-engine"` to root `Cargo.toml`.

- [ ] **Step 5: Verify builds across feature combinations**

```bash
cargo check -p assay-engine --no-default-features
cargo check -p assay-engine --no-default-features --features "workflow backend-postgres"
cargo check -p assay-engine --no-default-features --features "workflow backend-sqlite"
cargo check -p assay-engine --no-default-features --features "workflow auth dashboard backend-postgres"
cargo check -p assay-engine
```

All five expected clean.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/assay-engine
git commit -m "feat(engine): scaffold assay-engine crate + binary stub"
```

---

### Task 0.5: Move root `src/` binary to `crates/assay/`

This is the most invasive Phase 0 task. Everything under `/src`, `/stdlib`, and `/tests` becomes
`crates/assay/`. Root `Cargo.toml` loses its `[package]` and becomes workspace-only.

**Files:**

- Move: `src/` → `crates/assay/src/`
- Move: `stdlib/` → `crates/assay/stdlib/`
- Move: `tests/` → `crates/assay/tests/`
- Create: `crates/assay/Cargo.toml` (split from root)
- Modify: `/Cargo.toml` (strip `[package]`, keep workspace)
- Move: `/moon.yml` → `crates/assay/moon.yml` (with content tweaks for new paths)
- Create: new minimal `/moon.yml` (workspace root)
- Modify: `.moon/workspace.yml` (projects map)

- [ ] **Step 1: Capture baseline — count tests + note binary version**

```bash
cargo test --workspace 2>&1 | tail -5
./target/debug/assay --version
git status --short  # must be clean before the move
```

Record the test counts for later comparison.

- [ ] **Step 2: Create target directory + move files in one atomic commit**

```bash
mkdir -p crates/assay
git mv src crates/assay/src
git mv stdlib crates/assay/stdlib
git mv tests crates/assay/tests
git mv moon.yml crates/assay/moon.yml
```

- [ ] **Step 3: Author `crates/assay/Cargo.toml`**

Contents come from the existing root `[package]`, `[[bin]]`, `[lib]`, `[features]`,
`[dependencies]`, `[dev-dependencies]`, `[profile.release]` blocks. Key changes:

```toml
[package]
name = "assay-lua" # crates.io name stays — `assay` is squatted
version = "0.13.0" # bump to 0.13.0
edition = "2024"
# ... copy the rest of [package] from old root Cargo.toml

[[bin]]
name = "assay"
path = "src/main.rs"
required-features = ["cli"]

[lib]
name = "assay"
path = "src/lib.rs"

[features]
default = ["db", "server", "cli", "workflow"]
db = ["dep:sqlx"]
server = ["dep:http-body-util", "dep:hyper", "dep:hyper-util"]
cli = ["dep:clap", "dep:clap_complete", "dep:tracing-subscriber"]
# workflow feature now routes through assay-workflow's feature gates:
workflow = [
  "dep:assay-workflow",
  "assay-workflow/backend-postgres",
  "assay-workflow/backend-sqlite",
]

[dependencies]
# ... all the runtime deps as before ...
assay-workflow = { path = "../assay-workflow", version = "0.2", optional = true }
```

The `include_dir!` macro paths inside `src/` use `$CARGO_MANIFEST_DIR` which now resolves to
`crates/assay/` — so `include_dir!("$CARGO_MANIFEST_DIR/stdlib")` continues to work.

- [ ] **Step 4: Replace root `Cargo.toml` with workspace-only version**

```toml
[workspace]
resolver = "2"
members = [
  "crates/assay",
  "crates/assay-core",
  "crates/assay-auth",
  "crates/assay-dashboard",
  "crates/assay-engine",
  "crates/assay-workflow",
]

[workspace.package]
edition = "2024"
license = "Apache-2.0"
repository = "https://github.com/developerinlondon/assay"

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
strip = true
panic = "abort"
```

(Delete all `[package]`, `[[bin]]`, `[lib]`, `[features]`, `[dependencies]`, `[dev-dependencies]`
blocks from root — they now live at `crates/assay/Cargo.toml`.)

- [ ] **Step 5: Update `.moon/workspace.yml`**

```yaml
projects:
  assay-lua: "crates/assay"
  assay-core: "crates/assay-core"
  assay-workflow: "crates/assay-workflow"
  assay-dashboard: "crates/assay-dashboard"
  assay-auth: "crates/assay-auth"
  assay-engine: "crates/assay-engine"
  dashboard-e2e: "crates/assay-workflow/tests-e2e"
  site: "site"
  openclaw-extension: "openclaw-extension"
```

- [ ] **Step 6: Rewrite `crates/assay/moon.yml`**

Since moon's project-relative globs now resolve inside `crates/assay/`, update `fileGroups`:

```yaml
# crates/assay/moon.yml — runtime binary (assay-lua on crates.io, `assay`
# installed). Workspace-wide cargo tasks live here per existing pattern.

language: "rust"
layer: "application"
stack: "systems"

fileGroups:
  rust-sources:
    - "src/**/*"
    - "stdlib/**/*"
    - "Cargo.toml"
    - "../../Cargo.toml"
    - "../*/src/**/*"
    - "../*/Cargo.toml"

tasks:
  lint:
    command: "cargo clippy --workspace --tests -- -D warnings"
    inputs:
      - "@group(rust-sources)"
    options:
      cache: true
      runInCI: true

  build:
    command: "cargo build --release"
    inputs:
      - "@group(rust-sources)"
    outputs:
      - "../../target/release/assay"
    options:
      cache: true
      runInCI: true

  test:
    command: "cargo test --workspace"
    inputs:
      - "@group(rust-sources)"
      - "tests/**/*"
      - "../*/tests/**/*"
    options:
      cache: true
      runInCI: true
```

- [ ] **Step 7: Create minimal workspace-root `/moon.yml`**

Moon 2.x allows a workspace root project with no tasks; this documents that the root delegates.

```yaml
# /moon.yml — workspace root. Per-project moon.yml under crates/*
# owns actual tasks. This file exists so moon's project-graph
# resolves cleanly.

type: "library"
language: "rust"
```

- [ ] **Step 8: Full rebuild**

```bash
cargo clean
cargo build --workspace
```

Expected: clean build. Binary produced at `target/debug/assay`.

- [ ] **Step 9: Run the full test suite**

```bash
cargo test --workspace 2>&1 | tail -10
```

Expected: same pass/fail counts as baseline from Step 1. Any delta is a regression.

- [ ] **Step 10: Binary smoke test**

```bash
./target/debug/assay --version
# should print `assay 0.13.0`

./target/debug/assay run crates/assay/tests/fixtures/hello.lua
# or any existing fixture; output must match pre-move
```

- [ ] **Step 11: Moon smoke**

```bash
moon run assay-lua:build
moon run assay-lua:lint
moon run assay-lua:test
```

All three green.

- [ ] **Step 12: Commit**

```bash
git add -A
git commit -m "refactor: move root binary to crates/assay (workspace-only root)

Workspace root now has no [package]; crates/assay owns the assay-lua
crates.io package and the `assay` binary. Moon project graph updated.
No behaviour change for runtime consumers."
```

---

### Task 0.6: Add push-stream method signatures to `WorkflowStore` (signatures only)

Plan 10 requires `subscribe_runnable` and `subscribe_tasks` on the trait. Add them in Phase 0
(signatures + stub impls returning empty streams) so Phase 1's state refactor sees the trait in its
final shape.

**Files:**

- Modify: `crates/assay-workflow/src/store/mod.rs` (trait)
- Modify: `crates/assay-workflow/src/store/postgres.rs` (stub)
- Modify: `crates/assay-workflow/src/store/sqlite.rs` (stub)
- Modify: `crates/assay-workflow/Cargo.toml` (add `futures-core`, `futures-util`)
- Create: `crates/assay-workflow/tests/subscribe_trait_bounds.rs`

- [ ] **Step 1: Write the compile-time test first**

```rust
// crates/assay-workflow/tests/subscribe_trait_bounds.rs
//! Compile-time assertion that subscribe_runnable + subscribe_tasks
//! exist on WorkflowStore and produce Send streams.

use assay_workflow::store::WorkflowStore;
use futures_core::Stream;

fn _assert_runnable<S: WorkflowStore>(s: &S, ns: &str) {
    let _: std::pin::Pin<Box<dyn Stream<Item = String> + Send>> =
        Box::pin(s.subscribe_runnable(ns));
}

fn _assert_tasks<S: WorkflowStore>(s: &S, queues: &[&str]) {
    let _: std::pin::Pin<Box<dyn Stream<Item = String> + Send>> =
        Box::pin(s.subscribe_tasks(queues));
}
```

- [ ] **Step 2: Run — expect failure**

```bash
cargo check -p assay-workflow --tests
```

Expected: `method subscribe_runnable not found`.

- [ ] **Step 3: Add deps**

```toml
# crates/assay-workflow/Cargo.toml [dependencies]
futures-core = "0.3"
futures-util = "0.3"
```

- [ ] **Step 4: Add trait methods**

In `crates/assay-workflow/src/store/mod.rs`, add to the trait body:

```rust
    // ── Push subscriptions (hybrid wake-up) ───────────────────

    /// Emits workflow IDs as they become runnable in the given namespace.
    /// Backends without native push (SQLite) return an empty stream —
    /// the scheduler falls back to its local timer heap.
    fn subscribe_runnable(
        &self,
        namespace: &str,
    ) -> impl futures_core::Stream<Item = String> + Send + '_;

    /// Emits workflow task IDs as new tasks arrive on any of the listed
    /// queues. Backends without native push return an empty stream.
    fn subscribe_tasks<'a>(
        &'a self,
        queue_names: &'a [&'a str],
    ) -> impl futures_core::Stream<Item = String> + Send + 'a;
```

- [ ] **Step 5: Stub PG + SQLite impls**

In both `postgres.rs` and `sqlite.rs`, add to the `impl WorkflowStore` block:

```rust
    fn subscribe_runnable(
        &self,
        _namespace: &str,
    ) -> impl futures_core::Stream<Item = String> + Send + '_ {
        futures_util::stream::empty()
    }

    fn subscribe_tasks<'a>(
        &'a self,
        _queue_names: &'a [&'a str],
    ) -> impl futures_core::Stream<Item = String> + Send + 'a {
        futures_util::stream::empty()
    }
```

PG's real LISTEN/NOTIFY impl lands in Phase 3. SQLite stays empty by design (no cross-process push).

- [ ] **Step 6: Verify test passes**

```bash
cargo test -p assay-workflow --test subscribe_trait_bounds
```

Expected: PASS.

- [ ] **Step 7: Full workspace regression**

```bash
cargo test --workspace 2>&1 | tail -10
```

Expected: same pass/fail as baseline.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat(workflow): add subscribe_runnable + subscribe_tasks trait methods

Signatures only; PG and SQLite stub as empty streams. Real push impls
land in Phase 3 (SurrealDB via LIVE SELECT, PG via LISTEN/NOTIFY)."
```

---

### Phase 0 exit criteria

Run all, capture outputs in the branch's phase-0-baseline notes:

```bash
cargo build --workspace
cargo test --workspace
moon run :build
moon run :test
./target/debug/assay --version                      # 0.13.0
./target/debug/assay-engine                         # prints deferred message, exits 2
ls crates/                                          # 6 crate dirs
cat Cargo.toml | grep '\[package\]' && echo FAIL    # should produce no output
```

Zero regressions. All six crates present. Root is workspace-only.

---

## Phase 1 — State refactor

**What changes:** `Engine<S>` becomes `Engine` backed by `Arc<dyn WorkflowStore>`. `AppState<S>`
becomes `WorkflowCtx`. The api router moves from `Router<Arc<AppState<S>>>` to
`Router<WorkflowCtx>`. Types move to `assay-core`. Dashboard assets move to `assay-dashboard` with a
`DashboardCtx`.

**What doesn't change:** behaviour. Every existing integration test still passes without
modification.

**Sequencing:** the changes below must land in order — each unblocks the next and keeps the branch
compilable after every commit.

### Task 1.1: Move `types.rs` to `assay-core`

**Files:**

- Move: `crates/assay-workflow/src/types.rs` → `crates/assay-core/src/types/workflow.rs`
- Create: `crates/assay-core/src/types/mod.rs`
- Modify: `crates/assay-core/src/lib.rs`
- Modify: `crates/assay-core/Cargo.toml` (ensure deps are sufficient)
- Modify: `crates/assay-workflow/src/lib.rs` (re-export from core)
- Modify: `crates/assay-workflow/Cargo.toml` (add `assay-core` dep)

- [ ] **Step 1: Inspect types.rs for external dependencies**

```bash
grep -E '^use |^pub use' crates/assay-workflow/src/types.rs
```

Note every external dep. Typical suspects: `serde`, `serde_json`, `utoipa`, `chrono`. All present in
`assay-core/Cargo.toml` already.

- [ ] **Step 2: Move the file**

```bash
mkdir -p crates/assay-core/src/types
git mv crates/assay-workflow/src/types.rs crates/assay-core/src/types/workflow.rs
```

- [ ] **Step 3: Create types module root**

```rust
// crates/assay-core/src/types/mod.rs
pub mod workflow;
pub use workflow::*;
```

- [ ] **Step 4: Update core lib.rs**

```rust
// crates/assay-core/src/lib.rs
//! Shared types and storage traits used across assay crates.

pub mod types;
pub use types::*;
```

- [ ] **Step 5: Hook up the workflow crate's deps**

Edit `crates/assay-workflow/Cargo.toml`:

```toml
[dependencies]
assay-core = { path = "../assay-core", version = "0.1" }
```

- [ ] **Step 6: Update `assay-workflow/src/lib.rs`**

```rust
// crates/assay-workflow/src/lib.rs

// Was: pub mod types;
// Now:
pub use assay_core::types as types;
```

- [ ] **Step 7: Fix imports across the workflow crate**

Any `use crate::types::` imports in `engine.rs`, `scheduler.rs`, `api/*.rs`, `store/*.rs` — either
leave them (the re-export above preserves the path) OR update to `use assay_core::...`. Prefer
leaving intact to minimise diff.

- [ ] **Step 8: Verify**

```bash
cargo check -p assay-core
cargo check -p assay-workflow
cargo check --workspace
cargo test -p assay-workflow 2>&1 | tail -5
```

All green.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "refactor(core): move workflow types from assay-workflow to assay-core"
```

---

### Task 1.2: Move `WorkflowStore` trait to `assay-core`

**Files:**

- Modify: `crates/assay-workflow/src/store/mod.rs` (keep module declarations, move trait body out)
- Create: `crates/assay-core/src/store/mod.rs`
- Create: `crates/assay-core/src/store/workflow.rs`

- [ ] **Step 1: Move the trait body**

Create `crates/assay-core/src/store/workflow.rs` — move the full `pub trait WorkflowStore { ... }`
block from `crates/assay-workflow/src/store/mod.rs` into this new file.

Also move the `ApiKeyRecord`, `NamespaceRecord`, `NamespaceStats`, `QueueStats` structs (currently
at the bottom of `store/mod.rs`, lines ~428–465) into `crates/assay-core/src/types/workflow.rs`.
They are shared DTOs, not trait code.

- [ ] **Step 2: Create `crates/assay-core/src/store/mod.rs`**

```rust
pub mod workflow;
pub use workflow::WorkflowStore;
```

- [ ] **Step 3: Update `crates/assay-core/src/lib.rs`**

```rust
pub mod store;
pub mod types;

pub use store::WorkflowStore;
pub use types::*;
```

- [ ] **Step 4: Shrink `crates/assay-workflow/src/store/mod.rs`**

```rust
#[cfg(feature = "backend-postgres")]
pub mod postgres;

#[cfg(feature = "backend-sqlite")]
pub mod sqlite;

#[cfg(feature = "backend-surrealdb")]
pub mod surrealdb;

pub use assay_core::store::WorkflowStore;
pub use assay_core::{
    ApiKeyRecord, NamespaceRecord, NamespaceStats, QueueStats,
};
```

(Keep `surrealdb` feature-gated — its file is an empty stub until Phase 3.)

- [ ] **Step 5: Create stub `crates/assay-workflow/src/store/surrealdb.rs`**

```rust
//! SurrealDB backend. Full implementation lands in Phase 3 (plan 12b).

pub struct SurrealDbStore {
    _private: (),
}

impl SurrealDbStore {
    pub async fn connect(_dsn: &str) -> anyhow::Result<Self> {
        anyhow::bail!("SurrealDbStore::connect — not yet implemented (plan 12b Phase 3)")
    }
}

// The WorkflowStore impl lands in Phase 3.
```

- [ ] **Step 6: Gate sqlx feature in Cargo.toml**

```toml
# crates/assay-workflow/Cargo.toml
[features]
default = ["backend-postgres", "backend-sqlite"]

backend-postgres = ["dep:sqlx", "sqlx/postgres"]
backend-sqlite = ["dep:sqlx", "sqlx/sqlite"]
backend-surrealdb = ["dep:surrealdb"]

s3-archival = ["dep:aws-config", "dep:aws-sdk-s3"]

[dependencies]
sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "any"], optional = true }
surrealdb = { version = "3", default-features = false,
              features = ["protocol-ws", "protocol-http", "rustls"],
              optional = true }
# ... rest unchanged
```

- [ ] **Step 7: Update `crates/assay-workflow/src/lib.rs` re-exports**

```rust
#[cfg(feature = "backend-postgres")]
pub use store::postgres::PostgresStore;

#[cfg(feature = "backend-sqlite")]
pub use store::sqlite::SqliteStore;

#[cfg(feature = "backend-surrealdb")]
pub use store::surrealdb::SurrealDbStore;
```

- [ ] **Step 8: Per-feature verification**

```bash
cargo check -p assay-workflow --no-default-features --features backend-postgres
cargo check -p assay-workflow --no-default-features --features backend-sqlite
cargo check -p assay-workflow --no-default-features --features backend-surrealdb
cargo check -p assay-workflow
cargo check --workspace
cargo test -p assay-workflow 2>&1 | tail -5
```

All green.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "refactor(core): move WorkflowStore trait + shared DTOs to assay-core

Feature-gate PG / SQLite / SurrealDB backends in assay-workflow.
SurrealDbStore is a connect-only stub until Phase 3."
```

---

### Task 1.3: De-generalise `Engine<S>` to use `Arc<dyn WorkflowStore>`

**Goal:** remove the `S` type parameter from `Engine`, from handlers, from `AppState`. After this,
there's one handler signature per route, not N (one per backend).

**Files:**

- Modify: `crates/assay-workflow/src/engine.rs` (biggest file — 1397 lines)
- Modify: `crates/assay-workflow/src/scheduler.rs`
- Modify: `crates/assay-workflow/src/archival.rs`
- Modify: `crates/assay-workflow/src/dispatch_recovery.rs`
- Modify: `crates/assay-workflow/src/timers.rs`
- Modify: `crates/assay-workflow/src/state.rs`
- Modify: `crates/assay-workflow/src/health.rs`
- Modify: `crates/assay-workflow/src/api/**/*.rs` (handlers — many files)

- [ ] **Step 1: Sketch the target `Engine` signature**

```rust
// crates/assay-workflow/src/engine.rs

use std::sync::Arc;
use assay_core::WorkflowStore;

pub struct Engine {
    pub store: Arc<dyn WorkflowStore>,
    // ... other fields that were already present
}

impl Engine {
    pub fn new(store: Arc<dyn WorkflowStore>) -> Self {
        Self { store, /* ... */ }
    }
    // All existing methods become non-generic. Replace `self.store: S` usage
    // with `self.store.as_ref()` where a `&dyn` is needed, or `&*self.store`.
}
```

- [ ] **Step 2: Delete generic parameter from `Engine` definition**

In `crates/assay-workflow/src/engine.rs`, change:

```rust
pub struct Engine<S: WorkflowStore> { store: S, /* ... */ }
impl<S: WorkflowStore> Engine<S> { /* ... */ }
```

to:

```rust
pub struct Engine { store: Arc<dyn WorkflowStore>, /* ... */ }
impl Engine { /* ... */ }
```

Inside method bodies, `&self.store` now yields `&Arc<dyn WorkflowStore>` — all trait method calls
still work (`self.store.create_workflow(...).await`).

- [ ] **Step 3: Cascade to callers — scheduler, archival, dispatch_recovery, timers**

Each of these likely holds `Arc<Engine<S>>` or similar. Replace with `Arc<Engine>`.

```bash
grep -rn 'Engine<' crates/assay-workflow/src/
```

Every hit gets the `<S>` removed.

- [ ] **Step 4: Fix `AppState<S>` in `crates/assay-workflow/src/api/mod.rs`**

```rust
pub struct AppState {
    pub engine: Arc<Engine>,
    pub event_tx: broadcast::Sender<events::BroadcastEvent>,
    pub auth_mode: AuthMode,
    pub binary_version: Option<&'static str>,
}

pub fn router(state: Arc<AppState>) -> Router {
    // ... no longer generic on S
}

fn api_v1_router() -> Router<Arc<AppState>> {
    // ... etc.
}
```

Delete every `<S: WorkflowStore>` and every `<S>` from handler/router signatures in `api/*.rs`.
There's a lot of files; grep + mechanical edit:

```bash
grep -rn 'S: WorkflowStore' crates/assay-workflow/src/api/
grep -rn 'AppState<S>' crates/assay-workflow/src/api/
grep -rn '<S>' crates/assay-workflow/src/api/
```

For each, strip the generic parameter.

- [ ] **Step 5: Update dashboard.rs**

`crates/assay-workflow/src/api/dashboard.rs`:

```rust
// Was:
pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> { ... }

// Now:
pub fn router() -> Router<Arc<AppState>> { ... }
```

- [ ] **Step 6: Update lib.rs re-exports**

```rust
// crates/assay-workflow/src/lib.rs
pub use engine::Engine;
// (not Engine<S> — no generic any more)
```

- [ ] **Step 7: Build + test**

```bash
cargo check --workspace
cargo test -p assay-workflow 2>&1 | tail -10
```

Expected: green, same test counts.

The `assay-lua` binary (at `crates/assay/src/main.rs` or wherever it constructs the engine) probably
does `Engine::<PostgresStore>::new(store)` today. Update to `Engine::new(Arc::new(store))`.

- [ ] **Step 8: Full binary smoke**

```bash
cargo build --release
./target/release/assay --version
./target/release/assay run crates/assay/tests/fixtures/hello.lua
```

Output must match pre-refactor.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "refactor(workflow): de-generalise Engine<S> to Arc<dyn WorkflowStore>

Removes the S type parameter cascade through Engine, AppState, router,
and every handler. Behaviour unchanged; trait-object dispatch replaces
compile-time monomorphisation. See plan 12 § Architecture principle 2."
```

---

### Task 1.4: Rename `AppState` → `WorkflowCtx`; make it the workflow module's public state type

**Files:**

- Modify: `crates/assay-workflow/src/api/mod.rs`
- Modify: all `api/*.rs` (handler signatures)
- Modify: `crates/assay-workflow/src/lib.rs` (re-export `WorkflowCtx`)

This rename establishes the convention every future module follows: each module owns a
`pub struct {Module}Ctx`.

- [ ] **Step 1: Rename the struct**

```rust
// crates/assay-workflow/src/api/mod.rs

pub struct WorkflowCtx {
    pub engine: Arc<Engine>,
    pub event_tx: broadcast::Sender<events::BroadcastEvent>,
    pub auth_mode: AuthMode,
    pub binary_version: Option<&'static str>,
}

pub fn router(ctx: WorkflowCtx) -> Router {
    // construction stays similar, but now passes WorkflowCtx (owned,
    // not Arc) to with_state — Clone on WorkflowCtx means handlers
    // each get a cheap Arc bump via FromRef.
}
```

`WorkflowCtx` must derive `Clone` — that's the FromRef contract. Since every field is cheaply
cloneable (Arc, broadcast::Sender, Copy enum, Option<&'static str>), `#[derive(Clone)]` works.

```rust
#[derive(Clone)]
pub struct WorkflowCtx { /* ... */ }
```

- [ ] **Step 2: Update all handler State extractors**

`State<Arc<AppState>>` → `State<WorkflowCtx>`. Do this via grep + substitute:

```bash
grep -rn 'State<Arc<AppState>>' crates/assay-workflow/src/api/
grep -rn 'State<AppState>' crates/assay-workflow/src/api/
```

- [ ] **Step 3: Fix router state types**

`Router<Arc<AppState>>` → `Router<WorkflowCtx>`. Same grep pattern.

- [ ] **Step 4: Update router composition**

```rust
pub fn router(ctx: WorkflowCtx) -> Router {
    // The api_v1_router sub-functions return Router<WorkflowCtx>;
    // merge them and finish with .with_state(ctx).
    let authed = Router::new()
        .nest("/api/v1", api_v1_router())
        .nest("/api/v1", events::router());

    let public = Router::new().nest("/api/v1", public::router());

    authed
        .merge(public)
        .merge(dashboard::router())
        .merge(openapi::router())
        .with_state(ctx)
}
```

- [ ] **Step 5: Re-export from lib.rs**

```rust
pub use api::{router, WorkflowCtx};
```

- [ ] **Step 6: Update the runtime binary (`crates/assay/src/...`)**

Anywhere the runtime constructed `AppState`, rename to `WorkflowCtx`. The construction pattern
changes from `Arc::new(AppState { ... })` to `WorkflowCtx { ... }`.

- [ ] **Step 7: Verify**

```bash
cargo check --workspace
cargo test --workspace 2>&1 | tail -10
```

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor(workflow): rename AppState to WorkflowCtx (module-owned state)

Establishes the per-module Ctx convention. Future modules (auth,
dashboard, vault) follow the same pattern. Plan 12 § principle 1."
```

---

### Task 1.5: Extract dashboard asset bundle into `assay-dashboard`

This is the clean split we deferred in Task 0.3. Static assets + a dashboard-specific `DashboardCtx`
move to the new crate; the axum router for dashboard routes also moves.

**Files:**

- Move: `crates/assay-workflow/src/dashboard/` → `crates/assay-dashboard/assets/`
- Move: `crates/assay-workflow/src/api/dashboard.rs` → `crates/assay-dashboard/src/router.rs` (with
  reshaping)
- Move: `crates/assay-workflow/src/api/whitelabel.rs` → `crates/assay-dashboard/src/whitelabel.rs`
  (if it's dashboard-specific; check first)
- Modify: `crates/assay-dashboard/src/lib.rs`
- Modify: `crates/assay-workflow/src/api/mod.rs` (drop dashboard module, keep whitelabel import)

- [ ] **Step 1: Audit whitelabel.rs**

```bash
grep -rn 'whitelabel' crates/assay-workflow/src/
```

If non-dashboard code uses it (e.g. HTML error pages served by non-dashboard routes), keep it in
`assay-workflow` and let `assay-dashboard` import it. If only the dashboard uses it, move it too.

Determine the call graph and act accordingly. The rest of this task assumes whitelabel moves; adjust
if not.

- [ ] **Step 2: Move static assets**

```bash
mkdir -p crates/assay-dashboard/assets
git mv crates/assay-workflow/src/dashboard crates/assay-dashboard/assets/workflow
# resulting path: crates/assay-dashboard/assets/workflow/index.html,
#                 crates/assay-dashboard/assets/workflow/theme.css,
#                 crates/assay-dashboard/assets/workflow/styles/*.css,
#                 crates/assay-dashboard/assets/workflow/components/*.js
```

Prefixing with `workflow/` anticipates the `auth/` asset set that plan 11 adds.

- [ ] **Step 3: Define `DashboardCtx`**

```rust
// crates/assay-dashboard/src/lib.rs

pub mod assets;
pub mod router;

#[cfg(feature = "workflow")]
pub use router::workflow_router;

// DashboardCtx is intentionally small — the dashboard renders static
// assets + talks back to workflow/auth via their respective APIs.
#[derive(Clone, Default)]
pub struct DashboardCtx {
    pub branding: Branding,
}

#[derive(Clone, Debug)]
pub struct Branding {
    pub product_name: String,
    pub logo_url: Option<String>,
}

impl Default for Branding {
    fn default() -> Self {
        Self {
            product_name: "Assay".into(),
            logo_url: None,
        }
    }
}
```

- [ ] **Step 4: Typed asset bundle**

```rust
// crates/assay-dashboard/src/assets.rs

#[cfg(feature = "workflow")]
pub mod workflow {
    pub const INDEX_HTML: &str = include_str!("../assets/workflow/index.html");
    pub const THEME_CSS: &str = include_str!("../assets/workflow/theme.css");
    pub const STYLE_CSS: &str = concat!(
        include_str!("../assets/workflow/styles/00-base.css"),
        include_str!("../assets/workflow/styles/10-sidebar.css"),
        include_str!("../assets/workflow/styles/11-status-bar.css"),
        include_str!("../assets/workflow/styles/20-workflow-rows.css"),
        include_str!("../assets/workflow/styles/21-tables.css"),
        include_str!("../assets/workflow/styles/30-detail-panel.css"),
        include_str!("../assets/workflow/styles/40-modal.css"),
        include_str!("../assets/workflow/styles/41-row-actions.css"),
        include_str!("../assets/workflow/styles/42-select.css"),
        include_str!("../assets/workflow/styles/43-links.css"),
        include_str!("../assets/workflow/styles/50-pipeline.css"),
        include_str!("../assets/workflow/styles/51-events.css"),
        include_str!("../assets/workflow/styles/60-buttons.css"),
        include_str!("../assets/workflow/styles/61-forms.css"),
        include_str!("../assets/workflow/styles/62-cards.css"),
        include_str!("../assets/workflow/styles/63-toolbar.css"),
        include_str!("../assets/workflow/styles/70-feedback.css"),
        include_str!("../assets/workflow/styles/71-toast.css"),
        include_str!("../assets/workflow/styles/80-mobile.css"),
    );
    pub const APP_JS: &str = include_str!("../assets/workflow/app.js");
    pub const WORKFLOWS_JS: &str = include_str!("../assets/workflow/components/workflows.js");
    pub const DETAIL_JS: &str = include_str!("../assets/workflow/components/detail.js");
    pub const SCHEDULES_JS: &str = include_str!("../assets/workflow/components/schedules.js");
    pub const WORKERS_JS: &str = include_str!("../assets/workflow/components/workers.js");
    pub const QUEUES_JS: &str = include_str!("../assets/workflow/components/queues.js");
    pub const SETTINGS_JS: &str = include_str!("../assets/workflow/components/settings.js");
    pub const MODAL_JS: &str = include_str!("../assets/workflow/components/modal.js");
    pub const ACTIONS_JS: &str = include_str!("../assets/workflow/components/actions.js");
    pub const SELECT_JS: &str = include_str!("../assets/workflow/components/select.js");
}
```

- [ ] **Step 5: Port the dashboard router**

```rust
// crates/assay-dashboard/src/router.rs

#[cfg(feature = "workflow")]
pub fn workflow_router() -> axum::Router<crate::DashboardCtx> {
    use crate::assets::workflow::*;
    use axum::routing::get;
    use axum::Router;

    Router::new()
        .route("/", get(|| async { axum::response::Redirect::to("/workflow/") }))
        .route("/workflow", get(|| async { axum::response::Redirect::to("/workflow/") }))
        .route("/workflow/", get(serve_index))
        .route("/workflow/schedules", get(serve_index))
        .route("/workflow/workers", get(serve_index))
        .route("/workflow/queues", get(serve_index))
        .route("/workflow/settings", get(serve_index))
        .route("/workflow/theme.css", get(serve_theme))
        .route("/workflow/style.css", get(serve_style))
        .route("/workflow/app.js", get(serve_app))
        // ... (one handler per asset; mirror the current api/dashboard.rs list)
}

#[cfg(feature = "workflow")]
async fn serve_index(State(ctx): State<crate::DashboardCtx>) -> impl IntoResponse {
    let html = ctx.branding.render_index(crate::assets::workflow::INDEX_HTML);
    // use crate::whitelabel:: ... for substitutions
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html)
}
// ... etc.
```

The concrete body of these handlers mirrors the current `api/dashboard.rs`. The key shift is
accepting `State<DashboardCtx>` instead of `State<Arc<AppState>>`.

- [ ] **Step 6: Drop the dashboard module from assay-workflow**

`crates/assay-workflow/src/api/mod.rs` — remove `pub mod dashboard;` and the
`.merge(dashboard::router())` line.

Actually, better: in Phase 8 the engine is the thing that merges module routers. For now (before the
engine binary exists), `assay-workflow` can still provide a convenience wrapper that composes its
own router + dashboard for runtime users. Keep the wrapper during Phase 1:

```rust
// crates/assay-workflow/src/lib.rs
#[cfg(feature = "dashboard")]
pub fn router_with_dashboard(ctx: WorkflowCtx, dash: assay_dashboard::DashboardCtx) -> axum::Router {
    use axum::routing::Router;
    // Axum's FromRef-based router composition requires a root state.
    // Here we wrap both into a temporary EngineState-like struct.
    #[derive(Clone)]
    struct Temp {
        wf: WorkflowCtx,
        dash: assay_dashboard::DashboardCtx,
    }
    impl axum::extract::FromRef<Temp> for WorkflowCtx {
        fn from_ref(t: &Temp) -> Self { t.wf.clone() }
    }
    impl axum::extract::FromRef<Temp> for assay_dashboard::DashboardCtx {
        fn from_ref(t: &Temp) -> Self { t.dash.clone() }
    }
    Router::new()
        .merge(router(ctx.clone()).with_state::<Temp>(/* need thought */))
        .merge(assay_dashboard::workflow_router().with_state::<Temp>(/* */))
        .with_state(Temp { wf: ctx, dash })
}
```

(That compose is intricate; more realistically the runtime continues to use
`assay_workflow::router(ctx)` without dashboard until Phase 8, OR the runtime also adopts a local
`Composed` state right now. Discuss with reviewer if the compose isn't trivial.)

**Simpler pragmatic path:** in Phase 1, leave the runtime binary without its dashboard (use
`cargo run -p assay-engine` for a dashboard if needed). Document this as a temporary degradation in
the commit message. Phase 8 restores full dashboard composition via the engine binary — and the
runtime binary either depends on assay-engine's state composition helper or ships without dashboard.

The simpler path is correct here. **Mark the runtime's dashboard as temporarily unavailable during
Phases 1–7**, restored at Phase 8.

- [ ] **Step 7: Add dashboard dep to assay-workflow (temporary bridging)**

```toml
# crates/assay-workflow/Cargo.toml
[dependencies]
assay-dashboard = { path = "../assay-dashboard", version = "0.1", features = ["workflow"], optional = true }

[features]
default = ["backend-postgres", "backend-sqlite"]
# dashboard feature is intentionally NOT default — runtime ships without
# it during Phases 1–7. assay-engine binary (Phase 8) does the compose.
dashboard = ["dep:assay-dashboard"]
```

- [ ] **Step 8: Verify**

```bash
cargo check --workspace
cargo test -p assay-workflow 2>&1 | tail -5
```

Expected: workflow tests green. Dashboard-specific tests may fail temporarily; mark them `#[ignore]`
with a comment pointing to Phase 8.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "refactor(dashboard): extract assay-dashboard crate with typed asset bundle

Static assets + axum router now live in assay-dashboard. DashboardCtx
is the module's state type; handlers extract via State<DashboardCtx>.

Runtime binary's dashboard is temporarily offline until Phase 8
(assay-engine binary) wires EngineState with FromRef composition.
Runtime tests that assert dashboard endpoints are #[ignore] with a
reference to phase 8."
```

---

### Task 1.6: PG `LISTEN/NOTIFY` implementation for push streams (optional 1-h add)

**Scope decision:** optional in Phase 1. Phase 3 has a parallel SurrealDB LIVE SELECT impl; landing
PG's here lets the scheduler gain push wake-ups for PG deployments without waiting for Phase 3.

Skip this task if Phase 1 is running long — it doesn't block anything else.

If landing:

- Write the integration test (testcontainers PG, insert a runnable workflow, assert a stream event
  appears within 2s).
- Implement `subscribe_runnable` and `subscribe_tasks` using `sqlx::postgres::PgListener`.
- Add a migration under `crates/assay-workflow/migrations/postgres/<ts>_listen_notify.sql` defining
  the INSERT/UPDATE triggers.
- Commit as `feat(workflow/pg): implement LISTEN/NOTIFY push streams`.

Full detail lives in plan 12b Task 3.8.

---

### Phase 1 exit criteria

```bash
cargo check --workspace
cargo test --workspace 2>&1 | tail -10

# Per-feature:
cargo check -p assay-workflow --no-default-features --features backend-postgres
cargo check -p assay-workflow --no-default-features --features backend-sqlite
cargo check -p assay-workflow --no-default-features --features backend-surrealdb

# No generics remain:
! grep -rn 'Engine<' crates/assay-workflow/src/  # should produce no hits
! grep -rn 'AppState<S>' crates/assay-workflow/src/
! grep -rn 'WorkflowStore + .static>' crates/assay-workflow/src/

# FromRef-ready: WorkflowCtx derives Clone
grep 'derive.*Clone.*WorkflowCtx\|WorkflowCtx.*Clone' \
  crates/assay-workflow/src/api/mod.rs
```

All green. Workflow crate has one monomorphised type per handler. Dashboard is its own crate. The
state refactor ready for Phase 8's `FromRef` composition.

---

## What Phase 0+1 does not do

- Does not add SurrealDB workflow methods (Phase 3).
- Does not add auth modules (Phases 4–7).
- Does not build the engine binary (Phase 8).
- Does not change the runtime's behaviour except: temporarily takes the runtime dashboard offline
  (restored in Phase 8).

## Coordination with Phase 2

Phase 2 (push streams + feature gates) is partially absorbed into Phase 0 (Task 0.6) and Phase 1
(Task 1.2). After 12a ships, plan 12b starts with Phase 3 (SurrealDB workflow backend) directly;
Phase 2's ceremonial work is already done.
