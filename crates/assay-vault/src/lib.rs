//! Vault module for `assay-engine` — Vault + 1pw/Bitwarden + biscuit-share
//! in one crate, composing alongside `assay-auth` and `assay-workflow`
//! into the engine binary.
//!
//! See `.claude/plans/17-v0.3.0-secrets-module.md` for the full scope,
//! locked-in design decisions, and per-phase deliverables.
//!
//! ## Surface
//!
//! | Module                   | Plan ref | Purpose                                                     |
//! | ------------------------ | -------- | ----------------------------------------------------------- |
//! | [`schema`]               | —        | DDL bootstrap + migration runner (PG + SQLite)              |
//! | [`ctx::VaultCtx`]        | —        | Composed state — engine plugs it into [`assay_engine`]      |
//! | [`error::VaultError`]    | —        | Top-level error → HTTP / Lua mapping                        |
//! | `kv` (Phase 1)           | S1       | KV v2 — versioned, server-decryptable ops secrets           |
//! | `transit` (Phase 1)      | S2       | Encrypt / decrypt without exposing key material             |
//! | `dynamic` (Phase 5)      | S3       | Short-lived service credentials (PG / AWS / GCP / K8s)      |
//! | `collections` (Phase 3)  | S4       | Bitwarden-aligned shared collections + items + folders      |
//! | `personal_vault` (P3)    | S4       | Per-user personal vault (auto-created on signup)            |
//! | `share` (Phase 4)        | S5       | Biscuit-attenuated share links, server-revocable            |
//! | `bitwarden_compat` (P7)  | S6       | BW-protocol shim — stock BW clients work as front-ends      |
//! | `sealing` (Phase 2)      | S7       | Master KEK protection (Shamir / cloud KMS / HSM)            |
//! | `audit` (Phase 2)        | S8       | Forward audit events to syslog / S3 / webhook               |
//! | `ha` (Phase 6)           | S9       | Leader-lease tightening for sub-10s failover                |
//!
//! ## Storage
//!
//! Tables live under the `vault.*` PG schema (or attached `vault`
//! SQLite database, default `./data/vault.db`). The migration runner
//! ([`schema::migrate_postgres`] / [`schema::migrate_sqlite`]) records
//! each applied version into `engine.migrations` under
//! `module = MODULE_NAME` so subsequent boots skip already-applied
//! versions. Migrations are idempotent — every CREATE uses
//! `IF NOT EXISTS`.
//!
//! ## Phase trail
//!
//! Phase 0 (this commit): crate scaffold, the full plan-17 schema for
//! both backends, a `VaultCtx` placeholder, smoke-tested round-trips.
//! No HTTP handlers, no Lua stdlib, no implementation logic yet — those
//! land in Phases 1-7.

pub mod crypto;
pub mod ctx;
pub mod error;
pub mod schema;

#[cfg(any(
    feature = "vault-sealing-kms",
    feature = "vault-dynamic-aws",
    feature = "vault-dynamic-gcp",
    feature = "vault-audit-forwarding",
))]
pub mod cloud;

#[cfg(feature = "vault-sealing-kms")]
pub mod sealing;

#[cfg(feature = "vault-bitwarden-compat")]
pub mod bitwarden_compat;

#[cfg(feature = "vault-collections")]
pub mod zanzibar;

#[cfg(feature = "vault-audit-forwarding")]
pub mod audit;

#[cfg(feature = "vault-kv")]
pub mod kv;

#[cfg(feature = "vault-transit")]
pub mod transit;

#[cfg(feature = "vault-collections")]
pub mod personal_vault;

#[cfg(feature = "vault-collections")]
pub mod collections;

#[cfg(feature = "vault-collections")]
pub mod items;

#[cfg(feature = "vault-share")]
pub mod share;

#[cfg(any(
    feature = "vault-dynamic-postgres",
    feature = "vault-dynamic-aws",
    feature = "vault-dynamic-gcp",
    feature = "vault-dynamic-kubernetes",
))]
pub mod dynamic;

pub mod router;

#[cfg(any(feature = "backend-postgres", feature = "backend-sqlite"))]
pub mod store;

pub use crypto::KekHandle;
pub use ctx::VaultCtx;
pub use error::{Result, VaultError};
#[cfg(feature = "vault-kv")]
pub use kv::{KvMeta, KvRead, KvRow, KvService, KvStore};
pub use schema::{MIGRATION_VERSION, MODULE_NAME};
#[cfg(feature = "vault-transit")]
pub use transit::{TransitKey, TransitService, TransitStore, TransitVersion};

/// Stable module name registered in `engine.modules` and used as the
/// schema/attach name on both backends. Engine boot inserts a row with
/// `name = MODULE_NAME` when the runtime signal flips this module on.
pub const fn module_name() -> &'static str {
    MODULE_NAME
}
