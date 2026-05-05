//! Shared POD types for the OIDC provider.
//!
//! Each table defined in [`crate::schema::PG_DDL_V4`] / [`crate::schema::SQLITE_DDL_V4`]
//! has a matching `pub struct` here so handlers and stores share the same
//! shape. Timestamps are `f64` seconds since UNIX epoch (matches the rest
//! of the auth schema).

use serde::{Deserialize, Serialize};

/// Token-endpoint authentication method per OIDC Core §9. The `none`
/// variant marks a public client (PKCE-only — no shared secret).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenAuthMethod {
    /// HTTP Basic header (the OIDC default).
    ClientSecretBasic,
    /// Form-encoded `client_secret` field.
    ClientSecretPost,
    /// No client authentication — public client; PKCE is mandatory.
    None,
    /// JWT bearer assertion signed with the client's registered key.
    /// Reserved for v0.2.0+; not exercised by the v0.2.0 test suite.
    PrivateKeyJwt,
}

impl TokenAuthMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClientSecretBasic => "client_secret_basic",
            Self::ClientSecretPost => "client_secret_post",
            Self::None => "none",
            Self::PrivateKeyJwt => "private_key_jwt",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "client_secret_basic" => Some(Self::ClientSecretBasic),
            "client_secret_post" => Some(Self::ClientSecretPost),
            "none" => Some(Self::None),
            "private_key_jwt" => Some(Self::PrivateKeyJwt),
            _ => None,
        }
    }
}

/// Registered consumer application — one row in `auth.oidc_clients`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OidcClient {
    pub client_id: String,
    /// Argon2id PHC string. `None` for public clients (PKCE-only); when
    /// present, [`crate::oidc_provider::token`] verifies presented secrets
    /// against this column with [`crate::password::PasswordHasher`].
    pub client_secret_hash: Option<String>,
    pub redirect_uris: Vec<String>,
    pub name: String,
    pub logo_url: Option<String>,
    pub token_endpoint_auth_method: TokenAuthMethod,
    pub default_scopes: Vec<String>,
    pub require_consent: bool,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    pub pkce_required: bool,
    pub backchannel_logout_uri: Option<String>,
    pub created_at: f64,
}

impl OidcClient {
    /// Convenience constructor with sensible defaults — the most common
    /// shape (confidential client, code + refresh, PKCE on, consent on).
    /// Caller still has to set `client_secret_hash` / `redirect_uris`.
    pub fn new(client_id: impl Into<String>, name: impl Into<String>, created_at: f64) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret_hash: None,
            redirect_uris: Vec::new(),
            name: name.into(),
            logo_url: None,
            token_endpoint_auth_method: TokenAuthMethod::ClientSecretBasic,
            default_scopes: vec!["openid".to_string()],
            require_consent: true,
            grant_types: vec![
                "authorization_code".to_string(),
                "refresh_token".to_string(),
            ],
            response_types: vec!["code".to_string()],
            pkce_required: true,
            backchannel_logout_uri: None,
            created_at,
        }
    }

    /// Whether `redirect_uri` matches the registered list verbatim. OIDC
    /// Core §3.1.2.1 mandates exact match — no prefix matching, no host
    /// promotion, no trailing-slash normalisation.
    pub fn redirect_matches(&self, redirect_uri: &str) -> bool {
        self.redirect_uris.iter().any(|u| u == redirect_uri)
    }

    /// Whether `grant` is in the client's registered grant_types list.
    pub fn allows_grant(&self, grant: &str) -> bool {
        self.grant_types.iter().any(|g| g == grant)
    }
}

/// Federated upstream provider — one row in `auth.upstream_providers`.
/// Mirrors [`crate::oidc::UpstreamProvider`] shape; this struct adds the
/// admin-facing fields (`display_name`, `icon_url`, `enabled`) that the
/// in-memory registry doesn't carry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpstreamProvider {
    pub slug: String,
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub display_name: String,
    pub icon_url: Option<String>,
    pub enabled: bool,
}

/// One issued (and not-yet-consumed) authorization code — row in
/// `auth.oidc_authorization_codes`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuthorizationCode {
    pub code: String,
    pub client_id: String,
    pub user_id: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub nonce: Option<String>,
    pub state: Option<String>,
    pub issued_at: f64,
    pub expires_at: f64,
    pub consumed: bool,
}

/// One issued refresh token — row in `auth.oidc_refresh_tokens`.
/// `token_hash` is SHA-256 hex of the bearer the consumer presents; the
/// bearer itself never round-trips the DB.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RefreshToken {
    pub token_hash: String,
    pub client_id: String,
    pub user_id: String,
    pub scopes: Vec<String>,
    pub issued_at: f64,
    pub expires_at: f64,
    pub revoked: bool,
}

/// One SSO session row — `auth.oidc_sessions`. The `sid` matches the
/// `sid` claim baked into the issued id_token so back-channel logout
/// can target a specific session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OidcSession {
    pub sid: String,
    pub user_id: String,
    pub client_id: String,
    pub assay_session_id: Option<String>,
    pub issued_at: f64,
    pub backchannel_logout_uri: Option<String>,
}

/// One per-(user, client) consent grant — row in `auth.oidc_consents`.
/// Used by [`crate::oidc_provider::authorize`] to skip the consent screen
/// when a prior grant exists and the requested scopes are a subset of
/// the granted ones.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsentGrant {
    pub user_id: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub granted_at: f64,
}

/// One in-flight upstream-federation login — row in
/// `auth.oidc_upstream_states`. Created by `start_upstream_login`,
/// consumed (and deleted) by `complete_upstream_login`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpstreamLoginState {
    pub state: String,
    pub provider_slug: String,
    pub nonce: String,
    pub pkce_verifier: String,
    pub return_to: Option<String>,
    pub created_at: f64,
    pub expires_at: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_auth_method_round_trip() {
        for m in [
            TokenAuthMethod::ClientSecretBasic,
            TokenAuthMethod::ClientSecretPost,
            TokenAuthMethod::None,
            TokenAuthMethod::PrivateKeyJwt,
        ] {
            assert_eq!(TokenAuthMethod::parse(m.as_str()), Some(m));
        }
        assert_eq!(TokenAuthMethod::parse("garbage"), None);
    }

    #[test]
    fn oidc_client_new_defaults_to_confidential_pkce() {
        let c = OidcClient::new("c1", "App", 1.0);
        assert_eq!(c.client_id, "c1");
        assert!(c.pkce_required);
        assert!(c.require_consent);
        assert_eq!(
            c.token_endpoint_auth_method,
            TokenAuthMethod::ClientSecretBasic
        );
        assert!(c.allows_grant("authorization_code"));
        assert!(c.allows_grant("refresh_token"));
        assert!(!c.allows_grant("client_credentials"));
    }

    #[test]
    fn redirect_matches_is_exact() {
        let mut c = OidcClient::new("c1", "App", 0.0);
        c.redirect_uris = vec!["https://app.example.com/cb".to_string()];
        assert!(c.redirect_matches("https://app.example.com/cb"));
        // Trailing slash differs — no match.
        assert!(!c.redirect_matches("https://app.example.com/cb/"));
        // Prefix match — no match.
        assert!(!c.redirect_matches("https://app.example.com/cb/extra"));
    }
}
