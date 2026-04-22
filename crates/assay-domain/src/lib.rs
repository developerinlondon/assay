//! Shared types and storage traits used across assay crates.
//!
//! Consumers: `assay-workflow`, `assay-auth`, `assay-engine`.
//! Backend impls live in the domain crates behind Cargo features.

pub mod store;
pub mod types;

pub use store::WorkflowStore;
pub use types::*;
