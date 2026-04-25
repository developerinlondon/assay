//! Dashboard — typed asset bundle + axum router composition.
//!
//! Feature flags:
//!  - `workflow` (default): workflow run lists, events, timers, retries
//!  - `auth`: user + session + Zanzibar + OIDC client registry views
//!
//! The engine console assets + router are always present (engine-core
//! is always running, so its console doesn't gate on a feature flag).

pub mod assets;
#[cfg(feature = "workflow")]
pub mod ctx;
#[cfg(feature = "workflow")]
pub mod router;
#[cfg(feature = "workflow")]
pub mod whitelabel;

#[cfg(feature = "auth")]
pub mod auth_router;

pub mod engine_router;

#[cfg(feature = "workflow")]
pub use ctx::DashboardCtx;
#[cfg(feature = "workflow")]
pub use router::router as workflow_router;
#[cfg(feature = "workflow")]
pub use whitelabel::{WhitelabelConfig, WHITELABEL};

#[cfg(feature = "auth")]
pub use auth_router::router as auth_router;

pub use engine_router::router as engine_router;
