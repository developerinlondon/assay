//! Backend stores for the vault module.
//!
//! Phase 0 ships migrate-only entrypoints for PG and SQLite — enough
//! to apply the schema cleanly and let the smoke test round-trip a
//! single row. Phase 1 adds the per-feature store traits (KvStore,
//! TransitStore, …) and their PG / SQLite implementations.

#[cfg(feature = "backend-postgres")]
pub mod postgres;

#[cfg(feature = "backend-sqlite")]
pub mod sqlite;
