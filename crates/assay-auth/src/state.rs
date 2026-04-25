//! Shared state types for auth router composition.
//!
//! The auth router is generic over a parent state `S`. The parent must
//! `FromRef`-provide an `AuthCtx` and an [`AdminApiKeys`] for admin
//! routes. The engine binary's `EngineState<S>` does both — tests use
//! [`AuthCtxWithAdmin`] for an in-process state.

use std::sync::Arc;

use axum::extract::FromRef;

use crate::ctx::AuthCtx;

/// Bearer token guard for `/admin/*` routes. Compared in constant time
/// against the configured admin keys list. Operators rotate keys via
/// the engine config (`auth.admin_api_keys`).
#[derive(Clone, Default)]
pub struct AdminApiKeys(pub Arc<Vec<String>>);

impl AdminApiKeys {
    /// Empty — no admin keys configured. Every admin route returns 401.
    pub fn empty() -> Self {
        Self(Arc::new(Vec::new()))
    }

    /// Build from a static slice — tests + simple programmatic setups.
    pub fn from_iter<I, S>(iter: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self(Arc::new(iter.into_iter().map(Into::into).collect()))
    }

    /// Whether `presented` matches any configured admin key. Constant-
    /// time bytewise compare — short-circuits on length difference.
    pub fn check(&self, presented: &str) -> bool {
        let presented = presented.as_bytes();
        for key in self.0.iter() {
            let key = key.as_bytes();
            if key.len() != presented.len() {
                continue;
            }
            let mut diff = 0u8;
            for (a, b) in key.iter().zip(presented.iter()) {
                diff |= a ^ b;
            }
            if diff == 0 {
                return true;
            }
        }
        false
    }

    /// Whether at least one admin key is configured.
    pub fn enabled(&self) -> bool {
        !self.0.is_empty()
    }
}

/// Convenience parent state for tests: `AuthCtx` + `AdminApiKeys`. The
/// engine uses its own `EngineState<S>` instead.
#[derive(Clone)]
pub struct AuthCtxWithAdmin {
    pub auth: AuthCtx,
    pub admin: AdminApiKeys,
}

impl AuthCtxWithAdmin {
    pub fn new(auth: AuthCtx) -> Self {
        Self {
            auth,
            admin: AdminApiKeys::empty(),
        }
    }

    pub fn with_admin_keys(mut self, admin: AdminApiKeys) -> Self {
        self.admin = admin;
        self
    }
}

impl FromRef<AuthCtxWithAdmin> for AuthCtx {
    fn from_ref(s: &AuthCtxWithAdmin) -> Self {
        s.auth.clone()
    }
}

impl FromRef<AuthCtxWithAdmin> for AdminApiKeys {
    fn from_ref(s: &AuthCtxWithAdmin) -> Self {
        s.admin.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_keys_constant_time_check() {
        let keys = AdminApiKeys::from_iter(["abc", "xyz"]);
        assert!(keys.check("abc"));
        assert!(keys.check("xyz"));
        assert!(!keys.check("abd"));
        assert!(!keys.check(""));
        assert!(!keys.check("abcd"));
    }

    #[test]
    fn admin_keys_empty_disables() {
        let keys = AdminApiKeys::empty();
        assert!(!keys.enabled());
        assert!(!keys.check("anything"));
    }

    #[test]
    fn admin_keys_enabled_when_populated() {
        let keys = AdminApiKeys::from_iter(["k"]);
        assert!(keys.enabled());
    }
}
