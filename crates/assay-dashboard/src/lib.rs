//! Dashboard — typed asset bundle + axum router composition.
//!
//! Three consoles are always compiled in (plan-15 slice 3): workflow,
//! auth, and engine-core. The engine binary mounts them all; runtime
//! visibility per console is gated by `engine.modules` rows.

pub mod assets;
pub mod auth_router;
pub mod ctx;
pub mod engine_router;
pub mod router;
pub mod whitelabel;

pub use auth_router::router as auth_router;
pub use ctx::DashboardCtx;
pub use engine_router::router as engine_router;
pub use router::router as workflow_router;
pub use whitelabel::{WHITELABEL, WhitelabelConfig};
