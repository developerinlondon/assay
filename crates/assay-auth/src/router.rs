//! Auth HTTP routers.
//!
//! Two distinct routers split by audience (per plan 15):
//!
//! - [`oidc_spec_router`] — OIDC-spec endpoints that the wider OIDC
//!   ecosystem expects under stable, well-known paths
//!   (`/.well-known/*`, `/authorize`, `/token`, `/userinfo`, `/revoke`,
//!   `/introspect`, `/logout`, `/oidc/upstream/*`). Mounted at `/auth`
//!   by the engine.
//! - [`engine_auth_router`] — engine-internal auth surface (`/login`,
//!   `/logout` (DELETE), `/whoami`, `/passkey/*`, `/admin/*`). Mounted
//!   under `/api/v1/engine/auth` by the engine — keeps operator-facing
//!   APIs in one consistent namespace.
//!
//! [`router`] is kept for backward source-compat (in-process tests +
//! older callers) and returns the *engine-auth* router (the surface a
//! plain-state harness wants for unit tests).
//!
//! Both routers are generic over a parent state `S` from which
//! `AuthCtx` (and any feature-gated sub-state) can be extracted via
//! `axum::extract::FromRef`. The engine wires `EngineState<S>` through
//! both seams; in-process tests substitute an `AuthCtx`-only state
//! directly.

use axum::Router;
use axum::extract::FromRef;

use crate::ctx::AuthCtx;

/// OIDC-spec router — the public, well-known surface required by the
/// OIDC + OAuth2 specs. Mounted at `/auth` by the engine binary so the
/// canonical paths land at `/auth/.well-known/...`, `/auth/authorize`,
/// `/auth/token`, etc.
///
/// Empty (returns a no-op router) when the `auth-oidc-provider`
/// feature is off — the engine still mounts it; OIDC-less builds just
/// expose nothing under `/auth`.
pub fn oidc_spec_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AuthCtx: FromRef<S>,
    crate::state::AdminApiKeys: FromRef<S>,
{
    let r = Router::new();
    #[cfg(feature = "auth-oidc-provider")]
    let r = r.merge(crate::oidc_provider::spec_router::<S>());
    let _ = ();
    r
}

/// Engine-internal auth router — login, logout, whoami, passkey
/// ceremonies, and admin (users, sessions, biscuit, JWKS, Zanzibar,
/// audit, OIDC clients, OIDC upstream). Mounted under
/// `/api/v1/engine/auth` by the engine binary.
pub fn engine_auth_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AuthCtx: FromRef<S>,
    crate::state::AdminApiKeys: FromRef<S>,
{
    let r = Router::new();
    #[cfg(feature = "auth-session")]
    let r = r.merge(crate::session::router::<S>());
    // Cross-cutting admin endpoints (users / sessions / zanzibar /
    // biscuit / jwks / audit). Always merged when the auth router is
    // built — the handlers themselves degrade gracefully (503) when
    // their underlying module isn't compiled in or wired up.
    let r = r.merge(crate::admin::router::<S>());
    // OIDC admin (clients + upstream) lives on the engine-internal
    // surface too — operator-only CRUD that's never called by the
    // OIDC spec flows.
    #[cfg(feature = "auth-oidc-provider")]
    let r = r.merge(crate::oidc_provider::admin_router::<S>());
    r
}

/// Backward-compat alias for [`engine_auth_router`]. Older callers
/// (and unit tests) used `assay_auth::router::router()` to get the
/// composed admin/login surface — that surface is now the
/// engine-auth router. The OIDC spec router is exposed via
/// [`oidc_spec_router`] separately.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AuthCtx: FromRef<S>,
    crate::state::AdminApiKeys: FromRef<S>,
{
    engine_auth_router::<S>()
}
