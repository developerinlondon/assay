//! `GET /upstreams` — public listing of enabled upstream identity
//! providers, used by the login page (`assay-dashboard`'s
//! `/auth/login`) to render one button per upstream without first
//! needing an admin key.
//!
//! Only the fields safe to expose pre-auth (slug + display_name +
//! icon_url) are returned. `client_secret`, `auth_params`, and
//! disabled rows never leave the server.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde::Serialize;

use crate::ctx::AuthCtx;

#[derive(Serialize)]
pub struct PublicUpstream {
    pub slug: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

pub async fn list_public(State(ctx): State<AuthCtx>) -> Response {
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => {
            // No oidc_provider configured → empty list rather than
            // 503 so a login page can still render its "no providers
            // configured" empty state cleanly.
            return (StatusCode::OK, Json(Vec::<PublicUpstream>::new())).into_response();
        }
    };
    let rows = match provider.upstream.list().await {
        Ok(rows) => rows,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "list upstream").into_response(),
    };
    let body: Vec<PublicUpstream> = rows
        .into_iter()
        .filter(|r| r.enabled)
        .map(|r| PublicUpstream {
            slug: r.slug,
            display_name: r.display_name,
            icon_url: r.icon_url,
        })
        .collect();
    (StatusCode::OK, Json(body)).into_response()
}
