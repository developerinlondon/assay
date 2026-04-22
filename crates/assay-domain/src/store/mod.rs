//! Storage traits. Backend impls live in the domain crates behind
//! Cargo features (see `assay-workflow/src/store/`, `assay-auth/src/store/`).

pub mod workflow;

pub use workflow::WorkflowStore;
