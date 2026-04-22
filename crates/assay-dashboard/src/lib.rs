//! Dashboard — typed asset bundle + axum router composition.
//!
//! Feature flags:
//!  - `workflow` (default): workflow run lists, events, timers, retries
//!  - `auth`: user + session + Zanzibar + OIDC client registry views

#[cfg(feature = "workflow")]
pub mod assets;
#[cfg(feature = "workflow")]
pub mod ctx;
#[cfg(feature = "workflow")]
pub mod router;
#[cfg(feature = "workflow")]
pub mod whitelabel;

#[cfg(feature = "workflow")]
pub use ctx::DashboardCtx;
#[cfg(feature = "workflow")]
pub use router::router as workflow_router;
#[cfg(feature = "workflow")]
pub use whitelabel::{WhitelabelConfig, WHITELABEL};
