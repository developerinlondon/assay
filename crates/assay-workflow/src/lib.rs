pub mod api;
pub mod archival;
pub mod dispatch_recovery;
pub mod engine;
pub mod health;
pub mod scheduler;
pub mod state;
pub mod store;
pub mod timers;

// Types live in assay-core; re-exported here so existing `crate::types::*`
// paths continue to resolve.
pub use assay_core::types;

pub use engine::WorkflowEngine;
pub use store::postgres::PostgresStore;
pub use store::sqlite::SqliteStore;
pub use store::WorkflowStore;

#[cfg(feature = "s3-archival")]
pub(crate) fn timestamp_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
