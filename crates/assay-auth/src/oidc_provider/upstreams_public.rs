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
    // Intersect with the live registry — a row may be `enabled` in the
    // store but missing from the registry (failed discovery at boot, or
    // an admin update that didn't sync). Surfacing such rows would
    // produce a "Sign in with X" button that 404s at the start
    // endpoint with `unknown upstream provider`.
    let registry = ctx.oidc.as_ref();
    let body: Vec<PublicUpstream> = rows
        .into_iter()
        .filter(|r| r.enabled)
        .filter(|r| {
            registry
                .map(|reg| reg.client(&r.slug).is_some())
                .unwrap_or(false)
        })
        .map(|r| PublicUpstream {
            slug: r.slug,
            display_name: r.display_name,
            icon_url: r.icon_url,
        })
        .collect();
    (StatusCode::OK, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oidc_provider::types::UpstreamProvider;

    fn provider(slug: &str, enabled: bool) -> UpstreamProvider {
        UpstreamProvider {
            slug: slug.into(),
            issuer: "https://accounts.google.com".into(),
            client_id: "id".into(),
            client_secret: "secret".into(),
            display_name: slug.into(),
            icon_url: None,
            enabled,
            scopes: vec![],
            auth_params: Default::default(),
        }
    }

    fn render(rows: Vec<UpstreamProvider>, registered: &[&str]) -> Vec<PublicUpstream> {
        // Reimplement the filter chain so we can exercise it without a
        // live AuthCtx. Keeps in lockstep with `list_public` above.
        let registered: std::collections::HashSet<_> = registered.iter().copied().collect();
        rows.into_iter()
            .filter(|r| r.enabled)
            .filter(|r| registered.contains(r.slug.as_str()))
            .map(|r| PublicUpstream {
                slug: r.slug,
                display_name: r.display_name,
                icon_url: r.icon_url,
            })
            .collect()
    }

    #[test]
    fn disabled_rows_filtered() {
        let rows = vec![provider("google", true), provider("github", false)];
        let out = render(rows, &["google", "github"]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].slug, "google");
    }

    #[test]
    fn unregistered_rows_filtered() {
        let rows = vec![provider("google", true), provider("orphan", true)];
        let out = render(rows, &["google"]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].slug, "google");
    }

    #[test]
    fn secrets_never_appear_in_serialized_form() {
        let row = provider("google", true);
        let out = render(vec![row], &["google"]);
        let json = serde_json::to_string(&out).unwrap();
        assert!(!json.contains("secret"));
        assert!(!json.contains("client_id"));
        assert!(!json.contains("auth_params"));
        assert!(!json.contains("\"issuer\""));
    }
}
