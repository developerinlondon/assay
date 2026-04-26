//! SQLite backend for the vault module.
//!
//! Phase 0: only the migrate entrypoint. The caller is expected to
//! have ATTACHed `data/vault.db` AS `vault` before invoking this —
//! mirrors the wiring that `assay-auth` and `assay-workflow` already
//! rely on for their attached databases.

use anyhow::Result;
use sqlx::SqlitePool;

/// Apply the vault-schema DDL idempotently. Caller must have already
/// ATTACHed the vault database as `vault`.
pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    crate::schema::migrate_sqlite(pool).await
}
