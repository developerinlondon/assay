//! Public `/api/v1/*` endpoints — callable without authentication even when
//! the server is configured with `--auth-issuer` or `--auth-api-key`.
//!
//! Two routes live here today:
//!
//!   - `GET /api/v1/health`  — static liveness/readiness probe. Used by
//!     Kubernetes kubelet, load balancers, and any in-cluster monitor
//!     without a bearer token.
//!
//!   - `GET /api/v1/version` — engine version + build profile. Used by
//!     the CLI, dashboard, and third-party monitors to identify the
//!     running build. Cheap, static, no sensitive data.
//!
//! Both are wired outside the auth middleware layer in `api/mod.rs`. The
//! handlers live here (rather than being inlined) so Utoipa can tag and
//! document them under a single `public` module.
use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use utoipa::ToSchema;

use crate::api::AppState;
use crate::store::WorkflowStore;

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> {
    Router::new()
        .route("/health", get(health_check))
        .route("/version", get(version))
}

#[utoipa::path(
    get, path = "/api/v1/health",
    tag = "public",
    responses((status = 200, description = "Engine health — always unauthenticated")),
)]
pub async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "assay-workflow",
    }))
}

#[derive(Serialize, ToSchema)]
pub struct VersionInfo {
    /// Semver of the user-facing binary (or the `assay-workflow` crate when
    /// no embedder version was supplied). Matches `assay --version`.
    pub version: &'static str,
    /// `release` or `debug`.
    pub build_profile: &'static str,
}

#[utoipa::path(
    get, path = "/api/v1/version",
    tag = "public",
    responses((status = 200, description = "Engine version info", body = VersionInfo)),
)]
pub async fn version<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
) -> Json<VersionInfo> {
    let version = state
        .binary_version
        .unwrap_or(env!("CARGO_PKG_VERSION"));
    Json(VersionInfo {
        version,
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
    })
}
