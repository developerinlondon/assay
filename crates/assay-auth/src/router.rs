//! Auth HTTP router.
//!
//! Module routers (session, passkey, OIDC, Zanzibar) merge in here as
//! later phases land. The function is exposed unconditionally so engine
//! wiring (phase 8) can call `assay_auth::router::<EngineState<S>>()`
//! without `cfg`-guards at the call site — feature flags decide which
//! routes are present.
//!
//! Phase 8 makes the router generic over a parent state type `S` from
//! which `AuthCtx` (and any feature-gated sub-state) can be extracted
//! via `axum::extract::FromRef`. The engine wires `EngineState<S>`
//! through this seam; in-process tests substitute an `AuthCtx`-only
//! state directly.

use axum::Router;
use axum::extract::FromRef;

use crate::ctx::AuthCtx;

/// Auth routes. Mounted at engine root by `assay-engine`. Module routers
/// merge in here as their feature gates flip on.
///
/// Generic over a parent state `S` from which `AuthCtx` is extractable.
/// The simplest call site is `router::<AuthCtx>()` (identity FromRef);
/// the engine binary calls `router::<EngineState<_>>()` to thread the
/// composed state through.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AuthCtx: FromRef<S>,
    crate::state::AdminApiKeys: FromRef<S>,
{
    let r = Router::new();
    #[cfg(feature = "auth-oidc-provider")]
    let r = r.merge(crate::oidc_provider::router::<S>());
    #[cfg(feature = "auth-session")]
    let r = r.merge(crate::session::router::<S>());
    // Cross-cutting admin endpoints (users / sessions / zanzibar /
    // biscuit / jwks / audit). Always merged when the auth router is
    // built — the handlers themselves degrade gracefully (503) when
    // their underlying module isn't compiled in or wired up.
    
    r.merge(crate::admin::router::<S>())
}
