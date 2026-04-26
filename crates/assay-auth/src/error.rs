//! Error types for the auth crate.
//!
//! Backends and modules return [`Error`] uniformly so callers (engine
//! HTTP handlers in phase 8) can map a single enum to status codes.

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("session not found or expired")]
    SessionNotFound,
    #[error("csrf token mismatch")]
    CsrfMismatch,
    #[error("jwt verification failed: {0}")]
    Jwt(String),
    #[error("zanzibar depth limit exceeded")]
    ZanzibarDepth,
    #[error("zanzibar cycle detected")]
    ZanzibarCycle,
    #[error("oidc error: {0}")]
    Oidc(String),
    #[error("passkey error: {0}")]
    Passkey(String),
    #[error("backend: {0}")]
    Backend(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
