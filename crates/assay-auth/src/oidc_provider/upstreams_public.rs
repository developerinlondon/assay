//! `GET /upstreams` — public listing of enabled upstream identity
//! providers, used by the login page (`assay-dashboard`'s
//! `/auth/login`) to render one button per upstream without first
//! needing an admin key.
//!
//! Only the fields safe to expose pre-auth (slug + display_name +
//! icon_url) are returned. `client_secret`, `auth_params`, and
//! disabled rows never leave the server. `icon_url` is restricted to
//! `https://` to defang `javascript:` / `data:` payloads even though
//! the field is written by an authenticated admin.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde::Serialize;

use crate::ctx::AuthCtx;

use super::types::UpstreamProvider;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PublicUpstream {
    pub slug: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

/// Pure filter+projection that drives `list_public`. Split out so unit
/// tests exercise the same code path the live handler does — no
/// reimplementation drift.
///
/// * `enabled = false` rows are dropped.
/// * Rows whose slug is not in `registered` are dropped — the
///   in-memory registry may be missing entries the store lists
///   (failed boot-time discovery, or an admin update that didn't sync
///   yet). Surfacing them would produce buttons that 404 at the
///   start endpoint.
/// * `icon_url` is forced to `None` unless it parses as an `https://`
///   URL — defangs `javascript:`/`data:` even though the field is
///   admin-written.
pub fn filter_to_public(
    rows: Vec<UpstreamProvider>,
    mut registered: impl FnMut(&str) -> bool,
) -> Vec<PublicUpstream> {
    rows.into_iter()
        .filter(|r| r.enabled)
        .filter(|r| registered(r.slug.as_str()))
        .map(|r| PublicUpstream {
            slug: r.slug,
            display_name: r.display_name,
            icon_url: r.icon_url.and_then(sanitize_icon_url),
        })
        .collect()
}

/// Allow only `https://` icon URLs through to the browser. Anything
/// else (including `http://` for the standard mixed-content reason,
/// and any `javascript:` / `data:` payload an attacker-with-admin
/// might have planted) is stripped.
fn sanitize_icon_url(raw: String) -> Option<String> {
    let parsed = url::Url::parse(&raw).ok()?;
    if parsed.scheme() == "https" {
        Some(raw)
    } else {
        None
    }
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
    let registry = ctx.oidc.as_ref();
    let body = filter_to_public(rows, |slug| {
        registry
            .map(|reg| reg.client(slug).is_some())
            .unwrap_or(false)
    });
    (StatusCode::OK, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(slug: &str, enabled: bool) -> UpstreamProvider {
        UpstreamProvider {
            slug: slug.into(),
            issuer: "https://accounts.example.com".into(),
            client_id: "id".into(),
            client_secret: "secret".into(),
            display_name: slug.into(),
            icon_url: None,
            enabled,
            scopes: vec![],
            auth_params: Default::default(),
        }
    }

    fn with_icon(mut p: UpstreamProvider, icon: &str) -> UpstreamProvider {
        p.icon_url = Some(icon.into());
        p
    }

    #[test]
    fn disabled_rows_filtered() {
        let rows = vec![provider("google", true), provider("github", false)];
        let out = filter_to_public(rows, |_| true);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].slug, "google");
    }

    #[test]
    fn unregistered_rows_filtered() {
        let rows = vec![provider("google", true), provider("orphan", true)];
        let out = filter_to_public(rows, |slug| slug == "google");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].slug, "google");
    }

    #[test]
    fn secrets_never_appear_in_serialized_form() {
        let row = provider("google", true);
        let out = filter_to_public(vec![row], |_| true);
        let json = serde_json::to_string(&out).unwrap();
        assert!(!json.contains("secret"));
        assert!(!json.contains("client_id"));
        assert!(!json.contains("auth_params"));
        assert!(!json.contains("\"issuer\""));
    }

    #[test]
    fn https_icon_url_passes_through() {
        let row = with_icon(provider("google", true), "https://cdn.example.com/g.svg");
        let out = filter_to_public(vec![row], |_| true);
        assert_eq!(
            out[0].icon_url.as_deref(),
            Some("https://cdn.example.com/g.svg")
        );
    }

    #[test]
    fn javascript_data_and_http_icon_urls_stripped() {
        for bad in [
            "javascript:alert(1)",
            "data:image/svg+xml;base64,PHN2Zw==",
            "http://cdn.example.com/g.svg", // mixed-content; reject
            "//cdn.example.com/g.svg",
            "not a url",
        ] {
            let row = with_icon(provider("google", true), bad);
            let out = filter_to_public(vec![row], |_| true);
            assert_eq!(out[0].icon_url, None, "must strip {bad:?}");
        }
    }
}
