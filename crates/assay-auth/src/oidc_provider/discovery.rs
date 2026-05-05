//! `/.well-known/openid-configuration` — the OIDC discovery document.
//!
//! Per OpenID Connect Discovery 1.0 §4. We emit only the Core 1.0
//! profile fields plus the few extension fields v0.2.0 callers actually
//! need (`revocation_endpoint`, `end_session_endpoint`,
//! `introspection_endpoint`).

use axum::{Json, extract::State};
use serde_json::{Value, json};

use crate::ctx::AuthCtx;

/// Build the discovery JSON for `provider`. Pure function so the
/// snapshot tests + the runtime handler share one source of truth.
pub fn build_discovery(issuer: &str) -> Value {
    json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/authorize"),
        "token_endpoint": format!("{issuer}/token"),
        "userinfo_endpoint": format!("{issuer}/userinfo"),
        "jwks_uri": format!("{issuer}/.well-known/jwks.json"),
        "revocation_endpoint": format!("{issuer}/revoke"),
        "introspection_endpoint": format!("{issuer}/introspect"),
        "end_session_endpoint": format!("{issuer}/logout"),
        "scopes_supported": ["openid", "email", "profile", "offline_access"],
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["EdDSA"],
        "token_endpoint_auth_methods_supported": [
            "client_secret_basic",
            "client_secret_post",
            "none"
        ],
        "code_challenge_methods_supported": ["S256"],
        "claims_supported": [
            "sub", "email", "email_verified", "name", "preferred_username", "sid"
        ],
    })
}

/// `GET /.well-known/openid-configuration`.
pub async fn discovery_handler(State(ctx): State<AuthCtx>) -> Json<Value> {
    let issuer = ctx
        .oidc_provider
        .as_ref()
        .map(|p| p.issuer.as_str())
        .unwrap_or("");
    Json(build_discovery(issuer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_carries_required_fields() {
        let doc = build_discovery("https://idp.example.com");
        // Required by OpenID Connect Discovery 1.0 §4.
        for field in [
            "issuer",
            "authorization_endpoint",
            "token_endpoint",
            "jwks_uri",
            "response_types_supported",
            "subject_types_supported",
            "id_token_signing_alg_values_supported",
        ] {
            assert!(doc.get(field).is_some(), "missing {field}: {doc}");
        }
        assert_eq!(doc["issuer"], "https://idp.example.com");
        assert_eq!(
            doc["authorization_endpoint"],
            "https://idp.example.com/authorize"
        );
        assert_eq!(
            doc["jwks_uri"],
            "https://idp.example.com/.well-known/jwks.json"
        );
        assert_eq!(doc["subject_types_supported"], json!(["public"]));
        assert_eq!(
            doc["id_token_signing_alg_values_supported"],
            json!(["EdDSA"])
        );
        assert_eq!(doc["code_challenge_methods_supported"], json!(["S256"]));
    }

    #[test]
    fn discovery_lists_extension_endpoints() {
        let doc = build_discovery("https://idp.example.com");
        assert_eq!(doc["revocation_endpoint"], "https://idp.example.com/revoke");
        assert_eq!(
            doc["introspection_endpoint"],
            "https://idp.example.com/introspect"
        );
        assert_eq!(
            doc["end_session_endpoint"],
            "https://idp.example.com/logout"
        );
    }
}
