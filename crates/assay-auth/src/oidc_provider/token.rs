//! `/token` — OIDC token endpoint.
//!
//! Implements the `authorization_code` and `refresh_token` grants per
//! OIDC Core §3.1.3 / §12. The `client_credentials` grant is part of
//! the V1 surface (the trait shape leaves room) but the v0.2.0 plan
//! defers the implementation; the handler returns
//! `unsupported_grant_type` until phase 8 wires it.
//!
//! Bearer tokens (id_token / access_token) are EdDSA-signed JWTs minted
//! through the existing [`crate::jwt::JwtConfig`]. Refresh tokens are
//! opaque base64url strings — only their SHA-256 hash round-trips the
//! DB, the bearer itself is returned to the consumer once at issue
//! time.

use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::types::RefreshToken;

/// Default access_token / id_token lifetime — one hour.
pub const ACCESS_TOKEN_LIFETIME_SECS: f64 = 3600.0;
/// Default refresh_token lifetime — 30 days.
pub const REFRESH_TOKEN_LIFETIME_SECS: f64 = 60.0 * 60.0 * 24.0 * 30.0;

/// Form-encoded request body for `POST /token`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct TokenRequest {
    pub grant_type: String,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub code_verifier: Option<String>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
}

/// Successful response body. `expires_in` is seconds-from-now matching
/// the access_token's `exp` claim (RFC 6749 §5.1).
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
    pub id_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    pub scope: String,
}

/// Error response body — wire-compatible with OAuth 2 §5.2.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TokenErrorBody {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

/// Stable OAuth2 / OIDC token-endpoint error codes.
pub mod errors {
    pub const INVALID_REQUEST: &str = "invalid_request";
    pub const INVALID_CLIENT: &str = "invalid_client";
    pub const INVALID_GRANT: &str = "invalid_grant";
    pub const UNAUTHORIZED_CLIENT: &str = "unauthorized_client";
    pub const UNSUPPORTED_GRANT_TYPE: &str = "unsupported_grant_type";
    pub const INVALID_SCOPE: &str = "invalid_scope";
    pub const SERVER_ERROR: &str = "server_error";
}

/// Verify a PKCE `code_verifier` against the stored `code_challenge`
/// (S256 only). Returns `true` on match. Rejects empty inputs early.
pub fn verify_pkce_s256(verifier: &str, challenge: &str) -> bool {
    if verifier.is_empty() || challenge.is_empty() {
        return false;
    }
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let digest = hasher.finalize();
    let computed = data_encoding::BASE64URL_NOPAD.encode(&digest);
    constant_time_eq(computed.as_bytes(), challenge.as_bytes())
}

/// Constant-time bytewise compare. Length differences short-circuit
/// (this is fine — equal-length is the only meaningful side channel).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// SHA-256 hex of `token`. Used as the `auth.oidc_refresh_tokens.token_hash`
/// primary key — the bearer value never round-trips the DB in plaintext.
pub fn hash_refresh_token(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    let d = h.finalize();
    data_encoding::HEXLOWER.encode(&d)
}

/// Mint a fresh opaque refresh token. 64 bytes base64url ≈ 86 chars
/// (no padding), which is the typical opaque-token length consumer apps
/// expect.
pub fn mint_refresh_token() -> String {
    let mut buf = [0u8; 64];
    rand::rng().fill_bytes(&mut buf);
    format!("ort_{}", data_encoding::BASE64URL_NOPAD.encode(&buf))
}

/// Build a fresh [`RefreshToken`] row for `user_id` + `client_id`.
/// Caller separately stores the plaintext bearer to return in the
/// response — only the hash lives in the row.
pub fn build_refresh_row(
    user_id: &str,
    client_id: &str,
    scopes: &[String],
    plaintext: &str,
) -> RefreshToken {
    let now = now_secs();
    RefreshToken {
        token_hash: hash_refresh_token(plaintext),
        client_id: client_id.to_string(),
        user_id: user_id.to_string(),
        scopes: scopes.to_vec(),
        issued_at: now,
        expires_at: now + REFRESH_TOKEN_LIFETIME_SECS,
        revoked: false,
    }
}

/// Build the JWT claim object for an id_token.
///
/// `sid` ties the token to the SSO session row in `auth.oidc_sessions`
/// — back-channel logout uses it.
///
/// We emit per-scope optional claims:
/// - `email` scope → `email` + `email_verified`
/// - `profile` scope → `name`
pub fn build_id_token_claims(
    issuer: &str,
    user_id: &str,
    client_id: &str,
    sid: &str,
    scopes: &[String],
    nonce: Option<&str>,
    email: Option<&str>,
    email_verified: bool,
    name: Option<&str>,
) -> serde_json::Value {
    let now = now_secs();
    let mut claims = serde_json::json!({
        "iss": issuer,
        "sub": user_id,
        "aud": client_id,
        "iat": now as i64,
        "exp": (now + ACCESS_TOKEN_LIFETIME_SECS) as i64,
        "sid": sid,
    });
    if let Some(n) = nonce {
        claims["nonce"] = serde_json::Value::String(n.to_string());
    }
    if scopes.iter().any(|s| s == "email") {
        if let Some(e) = email {
            claims["email"] = serde_json::Value::String(e.to_string());
            claims["email_verified"] = serde_json::Value::Bool(email_verified);
        }
    }
    if scopes.iter().any(|s| s == "profile") {
        if let Some(n) = name {
            claims["name"] = serde_json::Value::String(n.to_string());
        }
    }
    claims
}

/// Build the JWT claim object for an access_token. Carries `client_id`
/// and `scope` so resource servers can authorize without an extra
/// lookup. Same `sid` as the id_token so revocation can fan out.
pub fn build_access_token_claims(
    issuer: &str,
    user_id: &str,
    client_id: &str,
    sid: &str,
    scopes: &[String],
) -> serde_json::Value {
    let now = now_secs();
    serde_json::json!({
        "iss": issuer,
        "sub": user_id,
        "aud": client_id,
        "client_id": client_id,
        "iat": now as i64,
        "exp": (now + ACCESS_TOKEN_LIFETIME_SECS) as i64,
        "sid": sid,
        "scope": scopes.join(" "),
        "token_use": "access",
    })
}

/// Mint a stable opaque session id for the SSO session row (`sid`
/// claim).
pub fn mint_sid() -> String {
    let mut buf = [0u8; 16];
    rand::rng().fill_bytes(&mut buf);
    format!("sid_{}", data_encoding::BASE64URL_NOPAD.encode(&buf))
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_s256_round_trip() {
        let verifier = "test-verifier-with-some-entropy-bytes";
        let mut h = Sha256::new();
        h.update(verifier.as_bytes());
        let challenge = data_encoding::BASE64URL_NOPAD.encode(&h.finalize());
        assert!(verify_pkce_s256(verifier, &challenge));
    }

    #[test]
    fn pkce_s256_rejects_wrong_verifier() {
        let mut h = Sha256::new();
        h.update(b"correct");
        let challenge = data_encoding::BASE64URL_NOPAD.encode(&h.finalize());
        assert!(!verify_pkce_s256("wrong", &challenge));
    }

    #[test]
    fn pkce_s256_rejects_empty() {
        assert!(!verify_pkce_s256("", "abc"));
        assert!(!verify_pkce_s256("abc", ""));
    }

    #[test]
    fn refresh_token_hash_is_deterministic() {
        let t = "ort_abcdef";
        assert_eq!(hash_refresh_token(t), hash_refresh_token(t));
        assert_ne!(hash_refresh_token(t), hash_refresh_token("ort_abcdeg"));
    }

    #[test]
    fn mint_refresh_token_starts_with_marker() {
        let t = mint_refresh_token();
        assert!(t.starts_with("ort_"));
        // base64url(64 bytes, no pad) = ceil(64*8/6) = 86 chars + 4 prefix
        assert!(t.len() >= 60);
    }

    #[test]
    fn build_refresh_row_hashes_plaintext_and_sets_expiry() {
        let plaintext = "ort_xyz";
        let row = build_refresh_row("u1", "c1", &["openid".to_string()], plaintext);
        assert_eq!(row.token_hash, hash_refresh_token(plaintext));
        assert_eq!(row.user_id, "u1");
        assert_eq!(row.client_id, "c1");
        assert!((row.expires_at - row.issued_at - REFRESH_TOKEN_LIFETIME_SECS).abs() < 1.0);
        assert!(!row.revoked);
    }

    #[test]
    fn id_token_claims_carry_required_oidc_fields() {
        let scopes = vec!["openid".to_string(), "email".to_string()];
        let v = build_id_token_claims(
            "https://idp.example.com",
            "user_alice",
            "client_app",
            "sid_x",
            &scopes,
            Some("nonce_y"),
            Some("alice@example.com"),
            true,
            None,
        );
        assert_eq!(v["iss"], "https://idp.example.com");
        assert_eq!(v["sub"], "user_alice");
        assert_eq!(v["aud"], "client_app");
        assert_eq!(v["sid"], "sid_x");
        assert_eq!(v["nonce"], "nonce_y");
        assert_eq!(v["email"], "alice@example.com");
        assert_eq!(v["email_verified"], true);
        assert!(v.get("name").is_none(), "no profile scope, no name");
    }

    #[test]
    fn id_token_claims_with_profile_scope_emits_name() {
        let scopes = vec!["openid".to_string(), "profile".to_string()];
        let v = build_id_token_claims(
            "https://idp",
            "u",
            "c",
            "sid",
            &scopes,
            None,
            None,
            false,
            Some("Alice Liddell"),
        );
        assert_eq!(v["name"], "Alice Liddell");
        assert!(v.get("email").is_none());
    }

    #[test]
    fn id_token_minimal_when_only_openid() {
        let scopes = vec!["openid".to_string()];
        let v = build_id_token_claims(
            "https://idp",
            "u",
            "c",
            "sid",
            &scopes,
            None,
            Some("dropped@example.com"),
            true,
            Some("dropped name"),
        );
        // Only openid scope, no email / name claims should leak through.
        assert!(v.get("email").is_none());
        assert!(v.get("email_verified").is_none());
        assert!(v.get("name").is_none());
    }

    #[test]
    fn access_token_claims_include_scope_string() {
        let scopes = vec!["openid".to_string(), "email".to_string()];
        let v = build_access_token_claims(
            "https://idp",
            "u",
            "c",
            "sid",
            &scopes,
        );
        assert_eq!(v["scope"], "openid email");
        assert_eq!(v["client_id"], "c");
        assert_eq!(v["token_use"], "access");
    }

    #[test]
    fn mint_sid_starts_with_marker() {
        let s = mint_sid();
        assert!(s.starts_with("sid_"));
    }
}
