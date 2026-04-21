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
pub mod server;
pub mod state;
