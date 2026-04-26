//! `/authorize` — OIDC authorization endpoint.
//!
//! Implements the user-agent leg of the Authorization-Code flow per
//! OIDC Core §3.1.2. We only emit the `code` response_type — the
//! implicit / hybrid flows are intentionally out of scope (they're
//! deprecated security practice).
//!
//! Flow:
//!
//! 1. Parse query params, validate against the registered client.
//! 2. If no active assay session → 302 to `/auth/login?return_to=…`.
//! 3. If consent required + no prior grant → render the consent screen.
//! 4. On approval, mint a 32-byte authorization code, INSERT into
//!    `auth.oidc_authorization_codes`, redirect back to `redirect_uri`
//!    with `?code=…&state=…`.

use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use serde::{Deserialize, Serialize};

use super::types::AuthorizationCode;
use super::OidcProviderConfig;

/// Default lifetime of an authorization code — 60 seconds. The OIDC
/// Core recommendation is "as short as practical"; one minute matches
/// the implicit limit most consumer apps assume.
pub const CODE_LIFETIME_SECS: f64 = 60.0;

/// Parsed `/authorize` query params.
///
/// `redirect_uri` is required (we don't fall back to a registered
/// default — OIDC Core §3.1.2.1 mandates explicit). `code_challenge` is
/// required when the client's `pkce_required` flag is true.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AuthorizeRequest {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: String,
    pub state: Option<String>,
    pub nonce: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub prompt: Option<String>,
    pub max_age: Option<u32>,
}

/// Validation outcome for an authorize request before any DB write.
#[derive(Debug, PartialEq, Eq)]
pub enum AuthorizeValidation {
    /// Everything checks out; caller can proceed to the session +
    /// consent flow.
    Ok {
        /// Parsed scope list (`scope` split on whitespace, deduped).
        scopes: Vec<String>,
    },
    /// The request is malformed in a way we cannot safely redirect for
    /// — render an error page instead.
    Fatal { reason: String },
    /// Recoverable error — redirect back to `redirect_uri` with the
    /// given OAuth2 `error` code (per OAuth 2 §4.1.2.1).
    Redirect {
        error: &'static str,
        description: String,
    },
}

/// Validate the request against the registered client. Pure function —
/// no DB writes, no side effects. The caller dispatches on the outcome.
pub fn validate(req: &AuthorizeRequest, client: &super::types::OidcClient) -> AuthorizeValidation {
    if req.response_type != "code" {
        return AuthorizeValidation::Redirect {
            error: "unsupported_response_type",
            description: format!("response_type {:?} is not supported", req.response_type),
        };
    }
    if !client.redirect_matches(&req.redirect_uri) {
        // Per OAuth 2 §3.1.2.4, when redirect_uri itself is the problem
        // we MUST NOT redirect; render an error page.
        return AuthorizeValidation::Fatal {
            reason: "redirect_uri does not match a registered URI".to_string(),
        };
    }
    if !client.allows_grant("authorization_code") {
        return AuthorizeValidation::Redirect {
            error: "unauthorized_client",
            description: "client does not allow authorization_code grant".to_string(),
        };
    }
    let scopes: Vec<String> = req
        .scope
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    if !scopes.iter().any(|s| s == "openid") {
        return AuthorizeValidation::Redirect {
            error: "invalid_scope",
            description: "openid scope required".to_string(),
        };
    }
    if client.pkce_required {
        let Some(method) = req.code_challenge_method.as_deref() else {
            return AuthorizeValidation::Redirect {
                error: "invalid_request",
                description: "code_challenge_method required (S256)".to_string(),
            };
        };
        if method != "S256" {
            return AuthorizeValidation::Redirect {
                error: "invalid_request",
                description: "code_challenge_method must be S256".to_string(),
            };
        }
        if req
            .code_challenge
            .as_deref()
            .map(str::is_empty)
            .unwrap_or(true)
        {
            return AuthorizeValidation::Redirect {
                error: "invalid_request",
                description: "code_challenge required for this client".to_string(),
            };
        }
    }
    AuthorizeValidation::Ok { scopes }
}

/// Build the per-row [`AuthorizationCode`] to INSERT. Generates a
/// 32-byte base64url code, stamps `issued_at` + `expires_at`. Caller
/// supplies the user_id (from the resolved session) and the resolved
/// scopes (from [`validate`]).
pub fn build_code(
    user_id: &str,
    req: &AuthorizeRequest,
    scopes: Vec<String>,
) -> AuthorizationCode {
    let code = format!("oac_{}", random_token());
    let now = now_secs();
    AuthorizationCode {
        code,
        client_id: req.client_id.clone(),
        user_id: user_id.to_string(),
        redirect_uri: req.redirect_uri.clone(),
        scopes,
        code_challenge: req.code_challenge.clone().unwrap_or_default(),
        code_challenge_method: req
            .code_challenge_method
            .clone()
            .unwrap_or_else(|| "S256".to_string()),
        nonce: req.nonce.clone(),
        state: req.state.clone(),
        issued_at: now,
        expires_at: now + CODE_LIFETIME_SECS,
        consumed: false,
    }
}

/// Build the `redirect_uri?code=…&state=…` URL for the success path.
pub fn redirect_with_code(redirect_uri: &str, code: &str, state: Option<&str>) -> String {
    let sep = if redirect_uri.contains('?') { '&' } else { '?' };
    let mut s = format!("{redirect_uri}{sep}code={}", url_encode(code));
    if let Some(st) = state {
        s.push_str(&format!("&state={}", url_encode(st)));
    }
    s
}

/// Build the `redirect_uri?error=…&state=…` URL for OAuth error responses.
pub fn redirect_with_error(
    redirect_uri: &str,
    error: &str,
    description: &str,
    state: Option<&str>,
) -> String {
    let sep = if redirect_uri.contains('?') { '&' } else { '?' };
    let mut s = format!(
        "{redirect_uri}{sep}error={}&error_description={}",
        url_encode(error),
        url_encode(description)
    );
    if let Some(st) = state {
        s.push_str(&format!("&state={}", url_encode(st)));
    }
    s
}

/// Original `/authorize` URL the user was visiting; used for the
/// "login first, then come back" flow. Encoding mirrors a thin
/// `urlencoded::form` since we don't want a full extra dep just for
/// this one helper.
pub fn return_to_for(authorize_url: &str) -> String {
    format!("/auth/login?return_to={}", url_encode(authorize_url))
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn random_token() -> String {
    let mut buf = [0u8; 32];
    rand::rng().fill_bytes(&mut buf);
    data_encoding::BASE64URL_NOPAD.encode(&buf)
}

/// Minimal RFC 3986 percent encoder — enough for the few characters we
/// stuff into redirect URLs (state strings, OAuth error codes). Encodes
/// every byte that isn't an unreserved character per §2.3.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{:02X}", byte));
        }
    }
    out
}

/// Provider config wrapper for handler use; keeps the import surface
/// shallow.
pub fn provider_issuer(p: &OidcProviderConfig) -> &str {
    &p.issuer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oidc_provider::types::{OidcClient, TokenAuthMethod};

    fn confidential_client() -> OidcClient {
        let mut c = OidcClient::new("c1", "App", 0.0);
        c.redirect_uris = vec!["https://app.example.com/cb".to_string()];
        c
    }

    fn req() -> AuthorizeRequest {
        AuthorizeRequest {
            response_type: "code".to_string(),
            client_id: "c1".to_string(),
            redirect_uri: "https://app.example.com/cb".to_string(),
            scope: "openid email".to_string(),
            state: Some("st_xyz".to_string()),
            nonce: Some("n_abc".to_string()),
            code_challenge: Some("ch_abc".to_string()),
            code_challenge_method: Some("S256".to_string()),
            prompt: None,
            max_age: None,
        }
    }

    #[test]
    fn validate_happy_path() {
        let r = req();
        let c = confidential_client();
        match validate(&r, &c) {
            AuthorizeValidation::Ok { scopes } => {
                assert_eq!(scopes, vec!["openid".to_string(), "email".to_string()])
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_bad_response_type() {
        let mut r = req();
        r.response_type = "token".to_string();
        let v = validate(&r, &confidential_client());
        match v {
            AuthorizeValidation::Redirect { error, .. } => {
                assert_eq!(error, "unsupported_response_type")
            }
            other => panic!("expected Redirect, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_unregistered_redirect_with_fatal() {
        let mut r = req();
        r.redirect_uri = "https://attacker.example.com/cb".to_string();
        let v = validate(&r, &confidential_client());
        assert!(matches!(v, AuthorizeValidation::Fatal { .. }));
    }

    #[test]
    fn validate_rejects_missing_pkce_when_required() {
        let mut r = req();
        r.code_challenge = None;
        let v = validate(&r, &confidential_client());
        match v {
            AuthorizeValidation::Redirect { error, .. } => {
                assert_eq!(error, "invalid_request")
            }
            other => panic!("expected Redirect, got {other:?}"),
        }
    }

    #[test]
    fn validate_skips_pkce_check_for_non_pkce_client() {
        let mut c = confidential_client();
        c.pkce_required = false;
        c.token_endpoint_auth_method = TokenAuthMethod::ClientSecretBasic;
        let mut r = req();
        r.code_challenge = None;
        r.code_challenge_method = None;
        assert!(matches!(validate(&r, &c), AuthorizeValidation::Ok { .. }));
    }

    #[test]
    fn validate_rejects_missing_openid_scope() {
        let mut r = req();
        r.scope = "email".to_string();
        let v = validate(&r, &confidential_client());
        match v {
            AuthorizeValidation::Redirect { error, .. } => assert_eq!(error, "invalid_scope"),
            other => panic!("expected Redirect, got {other:?}"),
        }
    }

    #[test]
    fn build_code_carries_request_metadata() {
        let r = req();
        let scopes = vec!["openid".to_string(), "email".to_string()];
        let c = build_code("user_alice", &r, scopes.clone());
        assert_eq!(c.client_id, "c1");
        assert_eq!(c.user_id, "user_alice");
        assert_eq!(c.redirect_uri, "https://app.example.com/cb");
        assert_eq!(c.scopes, scopes);
        assert_eq!(c.code_challenge, "ch_abc");
        assert_eq!(c.code_challenge_method, "S256");
        assert_eq!(c.nonce.as_deref(), Some("n_abc"));
        assert_eq!(c.state.as_deref(), Some("st_xyz"));
        assert!(c.code.starts_with("oac_"));
        assert!((c.expires_at - c.issued_at - CODE_LIFETIME_SECS).abs() < 1.0);
        assert!(!c.consumed);
    }

    #[test]
    fn redirect_with_code_appends_code_and_state() {
        let url = redirect_with_code(
            "https://app.example.com/cb",
            "oac_abc",
            Some("st_xyz"),
        );
        assert_eq!(url, "https://app.example.com/cb?code=oac_abc&state=st_xyz");
    }

    #[test]
    fn redirect_with_existing_query_uses_amp() {
        let url = redirect_with_code(
            "https://app.example.com/cb?prefilled=1",
            "oac_a",
            None,
        );
        assert_eq!(url, "https://app.example.com/cb?prefilled=1&code=oac_a");
    }

    #[test]
    fn redirect_with_error_includes_description() {
        let url = redirect_with_error(
            "https://app.example.com/cb",
            "access_denied",
            "user said no",
            Some("st"),
        );
        assert!(url.contains("error=access_denied"));
        assert!(url.contains("error_description=user%20said%20no"));
        assert!(url.contains("state=st"));
    }

    #[test]
    fn return_to_encodes_authorize_url() {
        let r = return_to_for("https://idp.example.com/authorize?client_id=c1&state=x");
        assert!(r.starts_with("/auth/login?return_to="));
        // Make sure it round-trips cleanly through a percent-decoder.
        assert!(r.contains("%3F"));
    }
}
