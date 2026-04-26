use thiserror::Error;

/// Top-level error for every vault-module path. Maps cleanly onto HTTP
/// responses in the admin layer (added in Phase 1) and onto Lua runtime
/// errors for the stdlib client.
#[derive(Debug, Error)]
pub enum VaultError {
    #[error("not found")]
    NotFound,

    #[error("forbidden")]
    Forbidden,

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("invalid input: {0}")]
    Invalid(String),

    #[error("crypto: {0}")]
    Crypto(String),

    /// The master KEK is sealed — the request can't proceed until an
    /// operator (or auto-unseal source) supplies enough unseal material.
    /// Every KV / transit / collection-key path checks for this before
    /// touching key material.
    #[error("sealed")]
    Sealed,

    #[error("backend: {0}")]
    Backend(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, VaultError>;
