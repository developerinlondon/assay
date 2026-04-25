//! Engine-core schema and bootstrap.
//!
//! Owns the four engine-scope tables introduced in v0.1.2:
//! `engine.modules`, `engine.audit`, `engine.instances`, `engine.migrations`.
//!
//! These tables are engine-core infrastructure and are always present
//! regardless of which functional modules are enabled. Module-specific
//! schemas (`workflow`, `auth`, …) are created/skipped based on
//! `engine.modules` enablement.
//!
//! ## Backend layout
//!
//! - **Postgres**: tables live in the `engine` schema, addressed
//!   schema-qualified (`engine.modules`, etc.).
//! - **SQLite**: tables live in an `engine.db` file attached as the
//!   `engine` database (Phase 3+); names match the PG layout exactly so
//!   queries are identical.

#[cfg(feature = "backend-postgres")]
pub mod pg;

#[cfg(feature = "backend-postgres")]
pub use pg::PgEngineSchema;

#[cfg(feature = "backend-sqlite")]
pub mod sqlite;

#[cfg(feature = "backend-sqlite")]
pub use sqlite::SqliteEngineSchema;

/// A row from the `engine.modules` table — the boot manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleRecord {
    pub name: String,
    pub enabled: bool,
    pub enabled_at: Option<f64>,
    pub enabled_by: Option<String>,
    pub version: Option<String>,
    pub config: serde_json::Value,
}
