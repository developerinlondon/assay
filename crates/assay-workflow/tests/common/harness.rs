//! Parametrised backend harness.
//!
//! Each test function is decorated with rstest cases for Postgres, SQLite,
//! and SurrealDB. The `Harness` enum wraps all three concrete store types
//! and delegates `WorkflowStore` calls through explicit `match` arms, so
//! test bodies remain backend-agnostic without requiring `dyn Trait`.
//!
//! The Surreal case fails until plan 12b Task 3.1 lands the real
//! `connect_full` implementation — that failure is intentional and drives
//! Phase 3's TDD cycle.

use assay_core::NamespaceRecord;
use assay_workflow::WorkflowStore;

// ── Harness ───────────────────────────────────────────────────────────────────

pub enum Harness {
    #[cfg(feature = "backend-postgres")]
    Postgres {
        _container: testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
        store: assay_workflow::PostgresStore,
    },
    #[cfg(feature = "backend-sqlite")]
    Sqlite {
        _tempdir: tempfile::TempDir,
        store: assay_workflow::SqliteStore,
    },
    #[cfg(feature = "backend-surrealdb")]
    Surreal {
        _container: testcontainers::ContainerAsync<testcontainers_modules::surrealdb::SurrealDb>,
        store: assay_workflow::SurrealDbStore,
    },
}

impl Harness {
    pub async fn list_namespaces(&self) -> anyhow::Result<Vec<NamespaceRecord>> {
        match self {
            #[cfg(feature = "backend-postgres")]
            Self::Postgres { store, .. } => store.list_namespaces().await,
            #[cfg(feature = "backend-sqlite")]
            Self::Sqlite { store, .. } => store.list_namespaces().await,
            #[cfg(feature = "backend-surrealdb")]
            Self::Surreal { store, .. } => store.list_namespaces().await,
        }
    }
}

// ── Backend selector ──────────────────────────────────────────────────────────

pub enum Backend {
    #[cfg(feature = "backend-postgres")]
    Postgres,
    #[cfg(feature = "backend-sqlite")]
    Sqlite,
    #[cfg(feature = "backend-surrealdb")]
    Surreal,
}

impl Backend {
    pub async fn setup(self) -> anyhow::Result<Harness> {
        match self {
            #[cfg(feature = "backend-postgres")]
            Self::Postgres => postgres_harness().await,
            #[cfg(feature = "backend-sqlite")]
            Self::Sqlite => sqlite_harness().await,
            #[cfg(feature = "backend-surrealdb")]
            Self::Surreal => surreal_harness().await,
        }
    }
}

// ── Per-backend setup ─────────────────────────────────────────────────────────

#[cfg(feature = "backend-postgres")]
async fn postgres_harness() -> anyhow::Result<Harness> {
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres as PgImage;

    let container = PgImage::default().start().await?;
    let host = container.get_host().await?;
    let port = container.get_host_port_ipv4(5432).await?;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    // The PG schema migration inserts "main" automatically; no need to call
    // create_namespace here — it would fail with a unique-key violation.
    let store = assay_workflow::PostgresStore::new(&url).await?;

    Ok(Harness::Postgres {
        _container: container,
        store,
    })
}

#[cfg(feature = "backend-sqlite")]
async fn sqlite_harness() -> anyhow::Result<Harness> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("assay.db");
    let url = format!("sqlite://{}?mode=rwc", path.display());

    // The SQLite schema migration inserts "main" automatically (INSERT OR IGNORE);
    // no need to call create_namespace here.
    let store = assay_workflow::SqliteStore::new(&url).await?;

    Ok(Harness::Sqlite {
        _tempdir: dir,
        store,
    })
}

#[cfg(feature = "backend-surrealdb")]
async fn surreal_harness() -> anyhow::Result<Harness> {
    use testcontainers::runners::AsyncRunner;
    use testcontainers::ImageExt;
    use testcontainers_modules::surrealdb::SurrealDb;

    // Pin to v3 — our surrealdb crate (3.x) speaks the v3 wire protocol.
    // testcontainers-modules 0.15 default image tag is v2.x, which causes
    // `Server sent no subprotocol` at handshake time.
    let container = SurrealDb::default()
        .with_tag("v3")
        .with_env_var("SURREAL_USER", "root")
        .with_env_var("SURREAL_PASS", "root")
        .start()
        .await?;
    let host = container.get_host().await?;
    let port = container.get_host_port_ipv4(8000).await?;
    let url = format!("ws://{host}:{port}");

    let store = assay_workflow::SurrealDbStore::connect_full(
        &url,
        "assay",
        "workflow",
        Some("root"),
        Some("root"),
    )
    .await?;

    Ok(Harness::Surreal {
        _container: container,
        store,
    })
}
