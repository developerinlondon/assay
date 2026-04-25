//! Composed auth context — the value engine state holds for the auth
//! module.
//!
//! Phase 4 wires user/session stores and (when JWT is enabled) the
//! [`crate::jwt::JwtConfig`]. Later phases extend this with the
//! Zanzibar store and OIDC provider registry. The struct is `Clone`
//! because axum's `FromRef` model requires it.

use std::sync::Arc;

use crate::store::{SessionStore, UserStore};

#[cfg(feature = "auth-jwt")]
use crate::jwt::JwtConfig;

#[derive(Clone)]
pub struct AuthCtx {
    /// Authoritative user record store. Carries password hashes,
    /// upstream-provider links, and (when phase 5 lands) passkeys.
    pub users: Arc<dyn UserStore>,
    /// Session record store — opaque session id + CSRF token + expiry.
    pub sessions: Arc<dyn SessionStore>,
    /// JWT issuance/verification configuration. Active key + history;
    /// see [`crate::jwt::JwtConfig`]. Present only when the
    /// `auth-jwt` feature is enabled.
    #[cfg(feature = "auth-jwt")]
    pub jwt: Option<JwtConfig>,
    // Phase 5 will add `oidc` provider registry here.
    // Phase 6 will add `zanzibar` store here.
}

impl AuthCtx {
    /// Construct a context from the bare minimum required by phase 4 —
    /// user and session stores. JWT and other modules are wired
    /// separately by callers when they're ready.
    pub fn new(users: Arc<dyn UserStore>, sessions: Arc<dyn SessionStore>) -> Self {
        Self {
            users,
            sessions,
            #[cfg(feature = "auth-jwt")]
            jwt: None,
        }
    }

    /// Replace the JWT configuration. Used by engine boot once the
    /// JWKS keys have been loaded from `auth.jwks_keys`.
    #[cfg(feature = "auth-jwt")]
    pub fn with_jwt(mut self, jwt: JwtConfig) -> Self {
        self.jwt = Some(jwt);
        self
    }
}
