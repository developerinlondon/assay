//! `/userinfo` — OIDC UserInfo endpoint per Core §5.3.
//!
//! Verifies a presented bearer access_token (signed by our JWKS),
//! re-loads the underlying user, and returns the scope-filtered claim
//! set as JSON.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::store::User;

/// Strip the `Bearer ` prefix from an `Authorization` header value.
/// Returns `None` if the header doesn't carry a Bearer token.
pub fn parse_bearer(header: &str) -> Option<&str> {
    let trimmed = header.trim_start();
    let prefix = "Bearer ";
    if !trimmed
        .get(..prefix.len())
        .map(|s| s.eq_ignore_ascii_case(prefix))
        .unwrap_or(false)
    {
        return None;
    }
    Some(trimmed[prefix.len()..].trim())
}

/// Claims we read off an access_token JWT for `/userinfo`. We only
/// pluck the fields we need; unrecognised claims are ignored.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AccessTokenClaims {
    pub sub: String,
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub sid: String,
}

impl AccessTokenClaims {
    /// Whitespace-split scope list.
    pub fn scopes(&self) -> Vec<String> {
        self.scope
            .split_whitespace()
            .map(|s| s.to_string())
            .collect()
    }
}

/// Build the userinfo response JSON for `user`, filtered by the scopes
/// the access_token was issued with. `sub` is always present (per
/// OIDC Core).
pub fn build_userinfo(user: &User, scopes: &[String]) -> Value {
    let mut out = json!({ "sub": user.id });
    if scopes.iter().any(|s| s == "email") {
        if let Some(email) = &user.email {
            out["email"] = Value::String(email.clone());
            out["email_verified"] = Value::Bool(user.email_verified);
        }
    }
    if scopes.iter().any(|s| s == "profile") {
        if let Some(name) = &user.display_name {
            out["name"] = Value::String(name.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> User {
        User {
            id: "user_alice".to_string(),
            email: Some("alice@example.com".to_string()),
            email_verified: true,
            display_name: Some("Alice".to_string()),
            created_at: 0.0,
        }
    }

    #[test]
    fn parse_bearer_handles_normal_header() {
        assert_eq!(parse_bearer("Bearer abc123"), Some("abc123"));
        assert_eq!(parse_bearer("bearer abc123"), Some("abc123"));
        assert_eq!(parse_bearer("BEARER abc123"), Some("abc123"));
    }

    #[test]
    fn parse_bearer_rejects_other_schemes() {
        assert_eq!(parse_bearer("Basic abc123"), None);
        assert_eq!(parse_bearer(""), None);
        assert_eq!(parse_bearer("Bearer"), None);
    }

    #[test]
    fn userinfo_minimal_when_only_openid_scope() {
        let v = build_userinfo(&user(), &["openid".to_string()]);
        assert_eq!(v["sub"], "user_alice");
        assert!(v.get("email").is_none());
        assert!(v.get("name").is_none());
    }

    #[test]
    fn userinfo_email_scope_emits_email_claims() {
        let scopes = vec!["openid".to_string(), "email".to_string()];
        let v = build_userinfo(&user(), &scopes);
        assert_eq!(v["email"], "alice@example.com");
        assert_eq!(v["email_verified"], true);
    }

    #[test]
    fn userinfo_profile_scope_emits_name() {
        let scopes = vec!["openid".to_string(), "profile".to_string()];
        let v = build_userinfo(&user(), &scopes);
        assert_eq!(v["name"], "Alice");
    }

    #[test]
    fn access_token_claims_scopes_helper() {
        let c = AccessTokenClaims {
            sub: "u".into(),
            aud: "c".into(),
            exp: 0,
            iat: 0,
            scope: "openid email".into(),
            client_id: "c".into(),
            sid: "s".into(),
        };
        assert_eq!(c.scopes(), vec!["openid".to_string(), "email".to_string()]);
    }
}
