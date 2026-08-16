//! `GET /v1/sys/health` — the one `sys/*` route the facade serves.
//!
//! Unauthenticated, like Vault's: k8s probes and ESO's store validation call
//! it without a token. A sealed engine answers 503, which is the status Vault
//! uses and what a probe needs to see.

use axum::Router;
use axum::extract::{FromRef, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::ctx::VaultCtx;

pub(super) fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    Router::new().route("/v1/sys/health", get(health::<S>))
}

async fn health<S>(State(vault): State<VaultCtx>) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let sealed = vault.seal_state.status().sealed;
    let status = if sealed {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    // The reported version is this crate's, not a Vault version: a client that
    // gates features on the Vault version string needs that check disabled.
    let body = serde_json::json!({
        "initialized": true,
        "sealed": sealed,
        "standby": false,
        "performance_standby": false,
        "replication_performance_mode": "disabled",
        "replication_dr_mode": "disabled",
        "server_time_utc": now_secs(),
        "version": env!("CARGO_PKG_VERSION"),
        "cluster_name": "assay-vault",
    });
    (status, axum::Json(body)).into_response()
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
