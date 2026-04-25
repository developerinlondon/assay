//! `/introspect` — RFC 7662 token introspection.
//!
//! Resource servers (any service that wants to validate an
//! access_token without verifying a JWT inline) POST `token` and a
//! client credential pair; the IdP responds with `{"active": true,
//! "client_id": …, "username": …, "scope": …, "exp": …}` (or
//! `{"active": false}` when the token is unknown / revoked / expired).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct IntrospectRequest {
    pub token: String,
    #[serde(default)]
    pub token_type_hint: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct IntrospectResponse {
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iat: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
}

impl IntrospectResponse {
    /// Sentinel "the token is not active" response per RFC 7662 §2.2 —
    /// returned for unknown / revoked / expired / wrong-aud tokens
    /// alike.
    pub fn inactive() -> Self {
        Self {
            active: false,
            client_id: None,
            username: None,
            scope: None,
            exp: None,
            sub: None,
            aud: None,
            iat: None,
            token_type: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inactive_response_serializes_to_just_active_false() {
        let v = serde_json::to_value(IntrospectResponse::inactive()).unwrap();
        assert_eq!(v, serde_json::json!({"active": false}));
    }

    #[test]
    fn active_response_round_trips_optional_fields() {
        let r = IntrospectResponse {
            active: true,
            client_id: Some("c1".into()),
            username: Some("alice".into()),
            scope: Some("openid email".into()),
            exp: Some(123),
            sub: Some("user_alice".into()),
            aud: Some("c1".into()),
            iat: Some(120),
            token_type: Some("Bearer".into()),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["active"], true);
        assert_eq!(v["client_id"], "c1");
        assert_eq!(v["scope"], "openid email");
    }
}
