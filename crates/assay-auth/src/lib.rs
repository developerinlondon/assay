//! Auth layer for assay-engine — OIDC client + provider, passkey,
//! Argon2 password, JWT, Biscuit capability tokens, session mgmt,
//! and Zanzibar-style ReBAC.
//!
//! Phase 4 (this commit) ships only the library surface that other
//! phases (5/6/7) and the engine binary (phase 8) compose. There are
//! NO HTTP handlers in this phase — just module code and store traits.
//!
//! Module boundaries and rationale: see plan 11. v0.1.2 alignment: see
//! plan 12c (top of file). Auth tables live in the `auth` schema (PG)
//! / attached `auth` database (SQLite, default `data/auth.db`); the
//! migration runner records each applied version in `engine.migrations`
//! under `module = 'auth'`.

pub mod error;

pub mod biscuit;
pub mod ctx;
pub mod router;
pub mod schema;
pub mod state;
pub mod store;

#[cfg(feature = "auth-session")]
pub mod session;

#[cfg(feature = "auth-password")]
pub mod password;

#[cfg(feature = "auth-jwt")]
pub mod jwt;

#[cfg(feature = "auth-oidc")]
pub mod oidc;

#[cfg(feature = "auth-oidc-provider")]
pub mod oidc_provider;

#[cfg(feature = "auth-passkey")]
pub mod passkey;

#[cfg(feature = "auth-zanzibar")]
pub mod zanzibar;

pub use ctx::AuthCtx;
pub use error::{Error, Result};
pub use router::router;
pub use schema::{MIGRATION_VERSION, MODULE_NAME};

/// Stable module name registered in `engine.modules` and used as the
/// schema/attach name on both backends. Engine boot inserts a row with
/// `name = MODULE_NAME` when `--enable=auth` (or equivalent runtime
/// signal) flips this module on.
pub const fn module_name() -> &'static str {
    MODULE_NAME
}
