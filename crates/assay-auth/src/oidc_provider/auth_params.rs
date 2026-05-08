//! `auth_params` whitelist — per-IdP authorize-URL parameter validation
//! and forwarding.
//!
//! Operators store a JSON object on `auth.upstream_providers.auth_params`
//! whose keys must be either in [`ALLOWED_KEYS`] or carry the `idp_`
//! prefix. Framework-owned keys (`client_id`, `redirect_uri`, `scope`,
//! `state`, `nonce`, `response_type`, `code_challenge*`, `request*`)
//! are always rejected — those are owned by the federation flow itself
//! and must never be admin-overridable.
//!
//! Values are strings ≤256 chars. Anything else (nested objects,
//! arrays, oversize strings) is rejected at write time so the
//! authorize-URL emitter never has to defend against bad shapes.

use std::collections::BTreeMap;

/// Keys explicitly forwarded to the upstream's authorize URL.
pub const ALLOWED_KEYS: &[&str] = &[
    "prompt",
    "login_hint",
    "domain_hint",
    "hd",
    "acr_values",
    "max_age",
    "ui_locales",
];

/// Prefix that opens up arbitrary IdP-specific params without a code
/// change for each new key.
pub const ALLOWED_PREFIX: &str = "idp_";

/// Keys the framework owns; must never appear in stored auth_params.
pub const REJECTED_KEYS: &[&str] = &[
    "client_id",
    "redirect_uri",
    "scope",
    "state",
    "nonce",
    "response_type",
    "code_challenge",
    "code_challenge_method",
    "request",
    "request_uri",
];

/// Maximum length of a single auth-param value, in bytes. URL-encoded
/// at use site, not at storage site.
pub const MAX_VALUE_LEN: usize = 256;

/// Reasons a single key/value pair fails validation. The admin handler
/// stringifies these into the per-key error body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthParamError {
    /// Key is in [`REJECTED_KEYS`] — owned by the framework.
    RejectedKey(String),
    /// Key is neither in [`ALLOWED_KEYS`] nor carries [`ALLOWED_PREFIX`].
    UnknownKey(String),
    /// Value exceeds [`MAX_VALUE_LEN`] bytes.
    ValueTooLong(String),
    /// Key contains characters that would break URL encoding (control
    /// chars, `=`, `&`, …). Belt-and-braces — admin should send clean
    /// keys to begin with, but a stored row that bypassed validation
    /// (e.g. via a manual SQL insert) shouldn't be allowed to break the
    /// authorize URL.
    InvalidKey(String),
}

impl std::fmt::Display for AuthParamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RejectedKey(k) => write!(f, "auth_params key {k:?} is owned by the framework"),
            Self::UnknownKey(k) => write!(f, "auth_params key {k:?} is not in the whitelist"),
            Self::ValueTooLong(k) => write!(
                f,
                "auth_params value for {k:?} exceeds {MAX_VALUE_LEN} chars"
            ),
            Self::InvalidKey(k) => write!(f, "auth_params key {k:?} contains invalid characters"),
        }
    }
}

impl std::error::Error for AuthParamError {}

/// Validate every key/value pair in `params`. Returns the first error
/// encountered (with key context) so callers can surface a per-key
/// HTTP 400 body. Caller iterates separately if it wants to collect
/// every error in one pass.
pub fn validate(params: &BTreeMap<String, String>) -> Result<(), AuthParamError> {
    for (k, v) in params {
        validate_pair(k, v)?;
    }
    Ok(())
}

/// Validate a single key/value pair. Exposed so the admin handler can
/// build a `Vec<(key, error_string)>` per the spec by iterating itself.
pub fn validate_pair(key: &str, value: &str) -> Result<(), AuthParamError> {
    if !is_clean_key(key) {
        return Err(AuthParamError::InvalidKey(key.to_string()));
    }
    if REJECTED_KEYS.contains(&key) {
        return Err(AuthParamError::RejectedKey(key.to_string()));
    }
    let allowed = ALLOWED_KEYS.contains(&key) || key.starts_with(ALLOWED_PREFIX);
    if !allowed {
        return Err(AuthParamError::UnknownKey(key.to_string()));
    }
    if value.len() > MAX_VALUE_LEN {
        return Err(AuthParamError::ValueTooLong(key.to_string()));
    }
    Ok(())
}

fn is_clean_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

/// Append every validated `auth_param` to a query-string buffer with
/// `&key=urlencoded(value)` form. Caller appends to an existing URL
/// that already carries the framework-owned params.
pub fn append_to_query(buf: &mut String, params: &BTreeMap<String, String>) {
    for (k, v) in params {
        if validate_pair(k, v).is_err() {
            // Defence in depth — a row that smuggled past validation
            // should not break the authorize URL silently. Skip it.
            continue;
        }
        buf.push('&');
        buf.push_str(k);
        buf.push('=');
        buf.push_str(&url_encode(v));
    }
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelisted_keys_pass() {
        for k in ALLOWED_KEYS {
            assert!(validate_pair(k, "x").is_ok(), "{k} should pass");
        }
    }

    #[test]
    fn idp_prefix_passes() {
        assert!(validate_pair("idp_login_hint_dom", "x").is_ok());
        assert!(validate_pair("idp_anything", "x").is_ok());
    }

    #[test]
    fn rejected_keys_are_rejected() {
        for k in REJECTED_KEYS {
            assert!(
                matches!(validate_pair(k, "x"), Err(AuthParamError::RejectedKey(_))),
                "{k} should be rejected as framework-owned"
            );
        }
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let err = validate_pair("totally_random", "v").unwrap_err();
        assert!(matches!(err, AuthParamError::UnknownKey(_)));
    }

    #[test]
    fn oversize_value_is_rejected() {
        let big = "x".repeat(MAX_VALUE_LEN + 1);
        let err = validate_pair("prompt", &big).unwrap_err();
        assert!(matches!(err, AuthParamError::ValueTooLong(_)));
    }

    #[test]
    fn boundary_value_passes() {
        let ok = "x".repeat(MAX_VALUE_LEN);
        assert!(validate_pair("prompt", &ok).is_ok());
    }

    #[test]
    fn invalid_key_chars_rejected() {
        assert!(matches!(
            validate_pair("with space", "v"),
            Err(AuthParamError::InvalidKey(_))
        ));
        assert!(matches!(
            validate_pair("", "v"),
            Err(AuthParamError::InvalidKey(_))
        ));
    }

    #[test]
    fn append_to_query_url_encodes_values() {
        let mut params = BTreeMap::new();
        params.insert("prompt".to_string(), "consent".to_string());
        params.insert("hd".to_string(), "example.com".to_string());
        let mut buf = String::new();
        append_to_query(&mut buf, &params);
        assert!(buf.contains("&prompt=consent"));
        assert!(buf.contains("&hd=example.com"));
    }

    #[test]
    fn append_to_query_skips_smuggled_invalid_pairs() {
        let mut params = BTreeMap::new();
        params.insert("client_id".to_string(), "evil".to_string());
        params.insert("prompt".to_string(), "consent".to_string());
        let mut buf = String::new();
        append_to_query(&mut buf, &params);
        assert!(!buf.contains("client_id"));
        assert!(buf.contains("prompt=consent"));
    }
}
