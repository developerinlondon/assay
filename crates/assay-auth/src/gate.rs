//! Unified authentication + authorization gate.
//!
//! [`extract_caller`] resolves a [`Caller`] from the request headers in
//! a fixed order: admin api-key (break-glass) → session cookie → JWT
//! bearer. Failure returns a ready-to-send `401 Unauthorized` response.
//!
//! [`require_role`] performs a coarse-grained Zanzibar role check on a
//! resolved caller. `AdminApiKey` callers bypass — the api-key list is
//! the operator's break-glass and is treated as carrying universal
//! authority by definition.
//!
//! [`require_role_for`] composes the two for the common case where the
//! caller doesn't need to be inspected separately.
//!
//! Used by:
//!
//! - [`crate::admin`] (`auth#system#admin`)
//! - [`crate::oidc_provider::admin`] (`auth#system#admin`)
//! - `assay_engine::engine_api` (`engine#core#admin`)
//! - `assay_engine::server`'s workflow gate middleware (`workflow#<ns>#access`)

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use serde_json::json;

use crate::ctx::AuthCtx;
use crate::state::AdminApiKeys;

/// An authenticated caller, produced by [`extract_caller`].
#[derive(Clone, Debug)]
pub struct Caller {
    /// Stable identifier for this caller. For session and JWT callers
    /// this is the user's id. For admin api-key callers this is a
    /// non-reversible token tail (e.g. `admin:****abc123`) safe to log.
    pub user_id: String,
    pub source: CallerSource,
}

/// Where the caller's identity proof came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallerSource {
    /// `assay_session` cookie resolved via [`crate::store::SessionStore`].
    SessionCookie,
    /// `Authorization: Bearer <jwt>` verified against the configured
    /// issuer's JWKS.
    Jwt,
    /// `Authorization: Bearer <key>` matched a configured admin
    /// api-key. Break-glass — bypasses Zanzibar role checks.
    AdminApiKey,
}

impl Caller {
    /// `true` iff the caller authenticated via the admin api-key
    /// fallback. Used by [`require_role`] to skip the Zanzibar lookup —
    /// admin api-keys are operator-controlled break-glass and carry
    /// universal authority by construction.
    pub fn is_break_glass(&self) -> bool {
        matches!(self.source, CallerSource::AdminApiKey)
    }
}

/// Resolve a [`Caller`] from the request headers.
///
/// Resolution order is fixed:
///
/// 1. `Authorization: Bearer <token>` matches a configured admin
///    api-key → [`CallerSource::AdminApiKey`] (break-glass).
/// 2. `Cookie: assay_session=<id>` resolves to a live session →
///    [`CallerSource::SessionCookie`].
/// 3. `Authorization: Bearer <jwt>` parses + verifies →
///    [`CallerSource::Jwt`].
/// 4. Otherwise → `Err(401)`.
///
/// The error variant is a boxed `Response` so callers can just
/// `return *r;` on failure without re-wrapping.
pub async fn extract_caller(
    headers: &HeaderMap,
    #[cfg_attr(
        not(any(feature = "auth-session", feature = "auth-jwt")),
        allow(unused_variables)
    )]
    ctx: &AuthCtx,
    keys: &AdminApiKeys,
) -> Result<Caller, Box<Response>> {
    // 1. Admin api-key — operator break-glass. Checked first so the
    //    expensive session/JWT round-trips are skipped when an admin is
    //    on the call.
    if let Some(token) = bearer_token(headers)
        && keys.enabled()
        && keys.check(token)
    {
        return Ok(Caller {
            user_id: short_admin_actor(token),
            source: CallerSource::AdminApiKey,
        });
    }

    // 2. Session cookie.
    #[cfg(feature = "auth-session")]
    if let Some(sid) = cookie_value(headers, crate::session::SESSION_COOKIE) {
        let mgr = crate::session::SessionManager::with_default_duration(ctx.sessions.clone());
        if let Ok(Some(s)) = mgr.resolve(&sid).await {
            return Ok(Caller {
                user_id: s.user_id,
                source: CallerSource::SessionCookie,
            });
        }
    }

    // 3. JWT bearer.
    #[cfg(feature = "auth-jwt")]
    if let Some(token) = bearer_token(headers)
        && let Some(jwt) = ctx.jwt.as_ref()
    {
        #[derive(serde::Deserialize)]
        struct SubClaim {
            sub: String,
        }
        if let Ok(td) = jwt.verify::<SubClaim>(token) {
            return Ok(Caller {
                user_id: td.claims.sub,
                source: CallerSource::Jwt,
            });
        }
    }

    Err(unauthorized("authentication required"))
}

/// Enforce a coarse-grained Zanzibar role check on `caller`.
///
/// `(object_type, object_id)` identifies the resource and `permission`
/// is the relation/permission name. `AdminApiKey` callers bypass.
///
/// Returns `Err(403)` on a denied check, `Err(500)` on a Zanzibar
/// backend error. With `auth-zanzibar` disabled at compile time every
/// non-break-glass caller fails closed with `403`.
pub async fn require_role(
    caller: &Caller,
    #[cfg_attr(not(feature = "auth-zanzibar"), allow(unused_variables))] ctx: &AuthCtx,
    #[cfg_attr(not(feature = "auth-zanzibar"), allow(unused_variables))] object_type: &str,
    #[cfg_attr(not(feature = "auth-zanzibar"), allow(unused_variables))] object_id: &str,
    #[cfg_attr(not(feature = "auth-zanzibar"), allow(unused_variables))] permission: &str,
) -> Result<(), Box<Response>> {
    if caller.is_break_glass() {
        return Ok(());
    }
    #[cfg(feature = "auth-zanzibar")]
    {
        use crate::zanzibar::{CheckResult, Consistency, ObjectRef, SubjectRef};
        let Some(store) = ctx.zanzibar.as_ref() else {
            // Zanzibar feature compiled in but not wired into AuthCtx —
            // fail closed so a misconfigured boot doesn't silently
            // grant access.
            return Err(forbidden("zanzibar store not configured"));
        };
        let resource = ObjectRef {
            object_type: object_type.to_string(),
            object_id: object_id.to_string(),
        };
        let subject = SubjectRef {
            subject_type: "user".to_string(),
            subject_id: caller.user_id.clone(),
            subject_rel: String::new(),
        };
        match store
            .check(&resource, permission, &subject, Consistency::Minimum)
            .await
        {
            Ok(CheckResult::Allowed { .. }) => Ok(()),
            Ok(_) => Err(forbidden("permission denied")),
            Err(e) => Err(internal(&format!("zanzibar check: {e}"))),
        }
    }
    #[cfg(not(feature = "auth-zanzibar"))]
    {
        Err(forbidden("authorization not compiled in"))
    }
}

/// Resolve caller + check role in one call. Returns the resolved
/// caller on success so handlers that want it for audit logging can
/// pluck it out.
pub async fn require_role_for(
    headers: &HeaderMap,
    ctx: &AuthCtx,
    keys: &AdminApiKeys,
    object_type: &str,
    object_id: &str,
    permission: &str,
) -> Result<Caller, Box<Response>> {
    let caller = extract_caller(headers, ctx, keys).await?;
    require_role(&caller, ctx, object_type, object_id, permission).await?;
    Ok(caller)
}

// =====================================================================
//   helpers
// =====================================================================

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer ").or_else(|| s.strip_prefix("bearer ")))
        .map(str::trim)
}

#[cfg(feature = "auth-session")]
fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for kv in raw.split(';') {
        let kv = kv.trim();
        if let Some((k, v)) = kv.split_once('=')
            && k == name
        {
            return Some(v.to_string());
        }
    }
    None
}

fn short_admin_actor(token: &str) -> String {
    let t = token.trim();
    if t.len() <= 6 {
        return format!("admin:****{t}");
    }
    let tail = &t[t.len() - 6..];
    format!("admin:****{tail}")
}

fn unauthorized(msg: &str) -> Box<Response> {
    Box::new(
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized", "error_description": msg})),
        )
            .into_response(),
    )
}

fn forbidden(msg: &str) -> Box<Response> {
    Box::new(
        (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "forbidden", "error_description": msg})),
        )
            .into_response(),
    )
}

fn internal(msg: &str) -> Box<Response> {
    Box::new(
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "server_error", "error_description": msg})),
        )
            .into_response(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_break_glass_only_for_api_key() {
        let c = Caller {
            user_id: "x".into(),
            source: CallerSource::AdminApiKey,
        };
        assert!(c.is_break_glass());
        let c = Caller {
            user_id: "x".into(),
            source: CallerSource::SessionCookie,
        };
        assert!(!c.is_break_glass());
        let c = Caller {
            user_id: "x".into(),
            source: CallerSource::Jwt,
        };
        assert!(!c.is_break_glass());
    }

    #[test]
    fn short_admin_actor_truncates_long_tokens() {
        assert_eq!(short_admin_actor("abcdef0123456789"), "admin:****456789");
    }

    #[test]
    fn short_admin_actor_handles_short_tokens() {
        assert_eq!(short_admin_actor("abc"), "admin:****abc");
    }
}
