//! Postgres backend for the vault module.
//!
//! Phase 0: only the migrate entrypoint. Re-exports the schema runner so
//! the engine boot path can call `assay_vault::store::postgres::migrate`
//! symmetrically with the auth crate's surface.

use anyhow::Result;
use sqlx::PgPool;

/// Apply the vault-schema DDL idempotently. Called by the engine boot
/// path when the `vault` module is enabled.
pub async fn migrate(pool: &PgPool) -> Result<()> {
    crate::schema::migrate_postgres(pool).await
}
