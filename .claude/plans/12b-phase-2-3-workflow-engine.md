# 12b — Phase 2 — Workflow trait cleanup + parametrised test harness

> Sub-plan of [12-v0.13.0-execution.md](./12-v0.13.0-execution.md). Prerequisites: phases 0 + 1 from
> [12a](./12a-phase-0-1-workspace-and-state.md).

> **STATUS — REV 2 (2026-04-22):** Phase 3 was "third workflow backend" (dropped per plan 12 rev 2).
> What remains is Phase 2: ensure `WorkflowStore` contract is complete across PG18 + SQLite, and
> land a parametrised test harness over both backends.

**Phase 2 goal:** ensure `WorkflowStore` contract is complete and parametrised testing is ready
across PG18 + SQLite. Most of Phase 2 was absorbed into Phase 1 (trait moved to `assay-core`,
push-stream signatures added, backends feature-gated). What remains is a parametrised test harness.

---

## Phase 2 — Workflow trait cleanup + parametrised test harness

### Task 2.1: Parametrised test harness across PG18 + SQLite

**Files:**

- Create: `crates/assay-workflow/tests/common/mod.rs`
- Create: `crates/assay-workflow/tests/common/harness.rs`
- Modify: `crates/assay-workflow/Cargo.toml` (dev-deps)

Each existing test gets annotated with `#[rstest]` cases for both backends. The harness gives one
`Arc<dyn WorkflowStore>` per backend via a testcontainer (PG18) or a tempdir (SQLite).

- [ ] **Step 1: Add dev-dependencies**

```toml
# crates/assay-workflow/Cargo.toml [dev-dependencies]
rstest = "0.26"
testcontainers = "0.27"
testcontainers-modules = { version = "0.15", features = ["postgres"] }
tokio = { version = "1", features = ["full"] }
tempfile = "3"
```

- [ ] **Step 2: Write the harness**

```rust
// crates/assay-workflow/tests/common/harness.rs
//! Parametrised backend harness. Each test fn is decorated with rstest
//! cases for PG18 and SQLite; the test body is generic over
//! `&dyn WorkflowStore`.

use std::sync::Arc;
use assay_core::WorkflowStore;

#[cfg(feature = "backend-postgres")]
use assay_workflow::PostgresStore;
#[cfg(feature = "backend-sqlite")]
use assay_workflow::SqliteStore;

pub enum Harness {
    #[cfg(feature = "backend-postgres")]
    Postgres {
        _container: testcontainers::ContainerAsync<
            testcontainers_modules::postgres::Postgres>,
        store: Arc<PostgresStore>,
    },
    #[cfg(feature = "backend-sqlite")]
    Sqlite {
        _tempdir: tempfile::TempDir,
        store: Arc<SqliteStore>,
    },
}

impl Harness {
    #[cfg(feature = "backend-postgres")]
    pub async fn postgres() -> anyhow::Result<Self> {
        use testcontainers::runners::AsyncRunner;
        use testcontainers_modules::postgres::Postgres as PgImage;

        // PG18 image for PG18 feature parity (uuidv7, skip-scan, AIO).
        let container = PgImage::default()
            .with_tag("18-alpine")
            .start()
            .await?;
        let host = container.get_host().await?;
        let port = container.get_host_port_ipv4(5432).await?;
        let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

        let store = PostgresStore::connect(&url).await?;
        store.create_namespace("main").await?;
        Ok(Self::Postgres { _container: container, store: Arc::new(store) })
    }

    #[cfg(feature = "backend-sqlite")]
    pub async fn sqlite() -> anyhow::Result<Self> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("assay.db");
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let store = SqliteStore::connect(&url).await?;
        store.create_namespace("main").await?;
        Ok(Self::Sqlite { _tempdir: dir, store: Arc::new(store) })
    }

    pub fn store(&self) -> Arc<dyn WorkflowStore> {
        match self {
            #[cfg(feature = "backend-postgres")]
            Self::Postgres { store, .. } => store.clone(),
            #[cfg(feature = "backend-sqlite")]
            Self::Sqlite { store, .. } => store.clone(),
        }
    }
}

pub enum Backend { Postgres, Sqlite }

impl Backend {
    pub async fn setup(self) -> anyhow::Result<Harness> {
        match self {
            #[cfg(feature = "backend-postgres")] Self::Postgres => Harness::postgres().await,
            #[cfg(feature = "backend-sqlite")]   Self::Sqlite   => Harness::sqlite().await,
            #[allow(unreachable_patterns)] _ => anyhow::bail!("backend feature disabled"),
        }
    }
}
```

```rust
// crates/assay-workflow/tests/common/mod.rs
pub mod harness;
pub use harness::*;
```

- [ ] **Step 3: Smoke test across backends**

```rust
// crates/assay-workflow/tests/smoke_backends.rs
mod common;
use common::{Backend, Harness};
use rstest::rstest;

#[rstest]
#[case::pg(Backend::Postgres)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test(flavor = "multi_thread")]
async fn namespace_roundtrip(#[case] backend: Backend) {
    let h = backend.setup().await.unwrap();
    let store = h.store();
    let list = store.list_namespaces().await.unwrap();
    assert!(list.iter().any(|n| n.name == "main"));
}
```

- [ ] **Step 4: Run — confirm both backends pass**

```bash
cargo test -p assay-workflow --test smoke_backends \
    --features "backend-postgres backend-sqlite" \
    -- --nocapture
```

Expected: both cases green.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "test(workflow): parametrised backend harness (pg18, sqlite)

Harness exposes PG18 testcontainer and SQLite tempdir variants;
existing integration tests migrate to #[rstest] per-backend cases
in follow-up commits."
```

---

### Phase 2 exit criteria

- Harness compiles and one smoke test runs.
- Both PG18 and SQLite cases green.
- PG18 testcontainer image is `postgres:18-alpine`, confirmed via `SELECT version()` in the harness
  setup returning 18.x.

---

## Phase 3 — REMOVED per plan 12 rev 2 (2026-04-22)

Phase 3 was "Third workflow backend" — 15 sub-tasks implementing `WorkflowStore` for an external
document store (native graph edges, live subscriptions, transaction snapshots). This phase is
**dropped** per plan 12 rev 2.

**What was kept:** the trait abstraction result. `WorkflowStore` (in `assay-core`) is backend-
agnostic — if a future plan wants to add a third backend, it lands as a pure addition: new
`src/store/<name>.rs`, new feature flag, extend the `Backend` enum, reuse the Phase 2 harness.

**What was dropped:** the 15 implementation tasks, the testcontainer wiring for the third backend,
15 hours of scope from v0.13.0, and ~60 parametrised test cases that would have added ~3× to the
compile budget without product benefit.

See plan 12 Revision log for the rationale (clean release build time 91 s → 281 s, peak compile RAM
1.28 GB → 3.67 GB, zero capability loss because PG18 + `pgvector` + recursive CTEs covers the
graph + document + vector + live-query space).

---

## Coordination with later phases

After Phase 2 ships (Task 2.1 + harness), plan 12c picks up with Phase 4 (auth primitives). There is
no longer a long pole between Phase 2 and Phase 4 — the auth work can start as soon as the harness
is green on both backends.
