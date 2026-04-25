//! Auth layer for `assay-engine` — a self-hosted, single-binary
//! **Ory replacement** for `assay-engine v0.2.0`.
//!
//! `assay-auth` packages every primitive a serious identity provider
//! needs into one crate that composes into [`crate::AuthCtx`] and is
//! mounted under `/auth` by the engine:
//!
//! | Module                 | Replaces                  | Purpose                                                     |
//! | ---------------------- | ------------------------- | ----------------------------------------------------------- |
//! | [`session`]            | Ory Kratos (sessions)     | Cookie + CSRF session manager (Argon2id-backed)             |
//! | [`password`]           | Ory Kratos (passwords)    | Argon2id PHC strings, peppered hashing                      |
//! | [`jwt`]                | Hydra (JWT)               | RS256 issue/verify with rotated JWKS                        |
//! | [`oidc`]               | Kratos (federation)       | OIDC **client** — log in via Google/Apple/GitHub/upstream   |
//! | [`oidc_provider`]      | Ory Hydra                 | Full OIDC **provider** — `/authorize`, `/token`, `/userinfo`, `/.well-known/*`, RFC 7009 revoke, RFC 7662 introspect, back-channel logout |
//! | [`passkey`]            | Kratos (WebAuthn)         | `webauthn-rs`-backed passkey register + auth ceremonies     |
//! | [`zanzibar`]           | Ory Keto / SpiceDB        | ReBAC tuples + recursive-CTE walk on PG18 + SQLite          |
//! | [`biscuit`]            | (Ory has nothing)         | Datalog-attenuable capability tokens — **always-on**        |
//! | [`store`]              | —                         | `UserStore` / `SessionStore` traits + PG / SQLite backends  |
//! | [`admin`]              | Ory Console (HTTP API)    | Cross-cutting admin endpoints (users, sessions, Zanzibar, …)|
//!
//! ## Why use `assay-auth` instead of Ory?
//!
//! - **One static binary** (`assay-engine`, ~9 MB stripped) replaces a
//!   stack of Kratos + Hydra + Keto + Oathkeeper containers. Same
//!   features, ~50× less RAM and one process to ship/log/restart.
//! - **Backend symmetry.** PG18 + SQLite are both first-class via
//!   feature flags. SQLite means a self-hosted single-tenant deployment
//!   needs no database server at all — unique vs Ory.
//! - **Biscuit out of the box.** Datalog-attenuable capability tokens
//!   that callers can scope down further without a server round-trip.
//!   Ory has nothing equivalent; this is a real differentiator.
//! - **Workflow + auth share storage.** Atomic transactions across
//!   `auth.users` ⇄ `workflow.workflows` (cross-schema FKs on PG, both
//!   attachments on SQLite) — signups can mint workflow records in one
//!   transaction. Splitting Ory + Temporal forces 2-phase commit.
//! - **Lua-scriptable.** Every auth surface is reachable from the
//!   `assay.auth` Lua stdlib module — operators can build login,
//!   admin, and federation flows in scripts that the runtime binary
//!   ships with.
//!
//! ## Getting started
//!
//! Compose `AuthCtx` into your axum state via `axum::extract::FromRef`
//! (the engine binary's `EngineState<S>` is the canonical recipe — see
//! [`assay_engine`] for the wiring). Out-of-the-box you'll need a
//! [`store::UserStore`] + [`store::SessionStore`]; the
//! [`store::PostgresUserStore`] / [`store::SqliteUserStore`] /
//! [`store::PostgresSessionStore`] / [`store::SqliteSessionStore`]
//! impls cover both backends.
//!
//! ```no_run
//! # async fn build() -> anyhow::Result<()> {
//! # let pool = sqlx::SqlitePool::connect("sqlite::memory:").await?;
//! use std::sync::Arc;
//! use assay_auth::AuthCtx;
//! use assay_auth::store::{SqliteUserStore, SqliteSessionStore};
//!
//! let users = SqliteUserStore::new(pool.clone()).into_dyn();
//! let sessions = SqliteSessionStore::new(pool.clone()).into_dyn();
//! let ctx = AuthCtx::new(users, sessions);
//! // ctx is now ready to be plugged into your Router via FromRef.
//! # Ok(()) }
//! ```
//!
//! For the full deployment shape (issuer, JWKS rotation, OIDC provider
//! discovery, biscuit root key bootstrap, passkey RP setup, Zanzibar
//! store) lean on `assay_engine::run` — it builds an `AuthCtx` from
//! `engine.toml`, runs the auth migration, and serves everything on one
//! port.
//!
//! ## Storage model
//!
//! All auth tables live in the `auth` schema (PG) or attached `auth`
//! database (SQLite, default `./data/auth.db`). The migration runner
//! ([`schema::migrate_postgres`] / [`schema::migrate_sqlite`]) records
//! each applied version in `engine.migrations` under `module = 'auth'`,
//! keyed by [`MIGRATION_VERSION`]. Migrations are idempotent — every
//! `CREATE` uses `IF NOT EXISTS`.
//!
//! ## Feature flags
//!
//! The default feature `auth` pulls in every module. Slim builds can opt
//! a la carte — see the per-module `#[cfg(feature = "...")]` gates
//! below. `backend-postgres` and `backend-sqlite` are independent and
//! both default-on; downstream binaries pick the one(s) they need.
//!
//! ## Phase trail
//!
//! Module boundaries and per-module rationale live in plan 11. v0.2.0
//! alignment (Ory-replacement scope, biscuit-built-in posture, schema
//! layout) lives in plan 12c §"v0.2.0 alignment".

pub mod error;

pub mod admin;
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
pub use router::{engine_auth_router, oidc_spec_router, router};
pub use schema::{MIGRATION_VERSION, MODULE_NAME};

/// Stable module name registered in `engine.modules` and used as the
/// schema/attach name on both backends. Engine boot inserts a row with
/// `name = MODULE_NAME` when `--enable=auth` (or equivalent runtime
/// signal) flips this module on.
pub const fn module_name() -> &'static str {
    MODULE_NAME
}
