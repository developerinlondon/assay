//! Auth HTTP router.
//!
//! Module routers (session, passkey, OIDC, Zanzibar) merge in here as
//! later phases land. The function is exposed unconditionally so engine
//! wiring (phase 8) can call `assay_auth::router()` without `cfg`-guards
//! at the call site — feature flags decide which routes are present.
//!
//! Phase 7: when `auth-oidc-provider` is on, the OIDC provider router
//! (discovery, JWKS, /authorize, /token, /userinfo, /revoke,
//! /introspect, /logout) is merged in at the root.

use axum::Router;

use crate::ctx::AuthCtx;

/// Auth routes. Mounted at engine root by `assay-engine`. Module routers
/// merge in here as their feature gates flip on.
pub fn router() -> Router<AuthCtx> {
    let r = Router::new();
    #[cfg(feature = "auth-oidc-provider")]
    let r = r.merge(crate::oidc_provider::router());
    r
}
