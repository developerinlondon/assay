//! `/revoke` — RFC 7009 token revocation.
//!
//! Form fields: `token` (required), `token_type_hint` (optional —
//! `refresh_token` or `access_token`). Per RFC 7009 §2.2 we MUST return
//! 200 even when the token is unknown / already revoked / wrong type —
//! refusing to leak whether the token was valid.
//!
//! Refresh-token revocation is a simple UPDATE on
//! `auth.oidc_refresh_tokens`. Access-token revocation is harder for
//! pure JWTs (the bearer is self-contained); we surface the API but
//! the actual JTI denylist is deferred per the plan note.

use serde::Deserialize;

/// Form-encoded body for `POST /revoke`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct RevokeRequest {
    pub token: String,
    /// `refresh_token` / `access_token` — optional hint, per RFC 7009 §2.1.
    #[serde(default)]
    pub token_type_hint: Option<String>,
}

/// What to attempt first based on the hint.
#[derive(Debug, PartialEq, Eq)]
pub enum HintedKind {
    Refresh,
    Access,
    Unknown,
}

impl HintedKind {
    pub fn from_hint(s: Option<&str>) -> Self {
        match s {
            Some("refresh_token") => Self::Refresh,
            Some("access_token") => Self::Access,
            _ => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hinted_kind_parsing() {
        assert_eq!(
            HintedKind::from_hint(Some("refresh_token")),
            HintedKind::Refresh
        );
        assert_eq!(
            HintedKind::from_hint(Some("access_token")),
            HintedKind::Access
        );
        assert_eq!(HintedKind::from_hint(Some("garbage")), HintedKind::Unknown);
        assert_eq!(HintedKind::from_hint(None), HintedKind::Unknown);
    }
}
