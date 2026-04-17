//! Engine metadata endpoints — version, build info.
//!
//! Exists so the CLI, the dashboard, and any third-party monitor can
//! discover which build they're talking to without parsing banner text
//! from server logs. `GET /api/v1/version` is meant to be cheap,
//! unauthenticated (when auth mode allows), and version-stamped via
//! `env!("CARGO_PKG_VERSION")` at compile time.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use utoipa::ToSchema;

use crate::api::AppState;
use crate::store::WorkflowStore;

#[derive(Serialize, ToSchema)]
pub struct VersionInfo {
    /// Semver of the `assay-workflow` crate at compile time
    /// (`CARGO_PKG_VERSION`). Matches `assay --version` for the binary.
    pub version: &'static str,
    /// `release` or `debug` — derived from `cfg!(debug_assertions)`.
    pub build_profile: &'static str,
}

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> {
    Router::new().route("/version", get(version))
}

#[utoipa::path(
    get,
    path = "/api/v1/version",
    tag = "meta",
    responses((status = 200, description = "Engine version info", body = VersionInfo)),
)]
pub async fn version<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
) -> Json<VersionInfo> {
    // Prefer the embedding binary's version if it was supplied (e.g. the
    // `assay` CLI passes its own CARGO_PKG_VERSION). Fall back to this
    // crate's version for embedders that didn't set it.
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
