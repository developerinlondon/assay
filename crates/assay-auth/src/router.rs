//! Auth HTTP router (placeholder).
//!
//! Phase 4 ships no routes — module routers (session, passkey, OIDC,
//! Zanzibar) are merged in here as phases 5/6/7 complete. The function
//! is exposed today so engine wiring (phase 8) can call
//! `assay_auth::router()` unconditionally; the empty router is harmless.

use axum::Router;

use crate::ctx::AuthCtx;

/// Auth routes. Mounted at engine root by `assay-engine`. Empty in
/// phase 4 — module routers merge in here as later phases land.
pub fn router() -> Router<AuthCtx> {
    Router::new()
}
