//! Session management — opaque server-side sessions backed by
//! [`crate::store::SessionStore`].
//!
//! Plan 11 reference: "auth.session" — an opaque session id (random 32
//! bytes, base64url) plus a parallel CSRF token. The cookie value is the
//! session id; the server resolves it against the store on every
//! request. Revocation is a single DELETE — no JWT-style "wait for
//! expiry" footgun.
//!
//! `SessionManager` is the entry point. Cookie helpers ([`cookie_for`],
//! [`csrf_cookie_for`]) build the standard cookie pair (HttpOnly +
//! Secure session cookie, JS-readable CSRF cookie) used by the
//! double-submit pattern.
//!
//! Phase 8 adds a HTTP router under [`router`] that mounts the
//! session-facing endpoints (`/login`, `/logout`, `/whoami`, passkey
//! ceremony). The auth top-level router merges this in.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cookie::{Cookie, SameSite, time::Duration as CookieDuration};
use rand::RngCore;
use url::Url;

use crate::error::Result;
use crate::store::{Session, SessionStore};

/// Cookie name carrying the opaque session id. HttpOnly — never read by
/// browser JS.
pub const SESSION_COOKIE: &str = "assay_session";

/// Cookie name carrying the CSRF token. NOT HttpOnly — client JS reads
/// this and echoes it in a request header (double-submit pattern).
pub const CSRF_COOKIE: &str = "assay_csrf";

/// Default session lifetime — 30 days. Matches typical "remember me"
/// expectations; per-deployment configuration overrides via
/// [`SessionManager::new`].
pub const DEFAULT_SESSION_DURATION: Duration = Duration::from_secs(60 * 60 * 24 * 30);

/// Owns the [`SessionStore`] and mints / resolves / revokes sessions.
///
/// Cheap to clone — the underlying store is reference-counted.
#[derive(Clone)]
pub struct SessionManager {
    store: Arc<dyn SessionStore>,
    default_duration: Duration,
}

impl SessionManager {
    /// Construct a manager with an explicit default session duration.
    /// Callers wanting the standard 30-day lifetime should use
    /// [`SessionManager::with_default_duration`].
    pub fn new(store: Arc<dyn SessionStore>, default_duration: Duration) -> Self {
        Self {
            store,
            default_duration,
        }
    }

    /// Construct with the [`DEFAULT_SESSION_DURATION`] (30 days).
    pub fn with_default_duration(store: Arc<dyn SessionStore>) -> Self {
        Self::new(store, DEFAULT_SESSION_DURATION)
    }

    /// Mint a fresh session for `user_id` and persist it via the store.
    /// The returned [`Session`] carries both the opaque cookie value
    /// (`id`) and the parallel CSRF token. Call sites set both cookies
    /// on the response (see [`cookie_for`] / [`csrf_cookie_for`]).
    pub async fn create(&self, user_id: &str) -> Result<Session> {
        let id = format!("sess_{}", random_token());
        let csrf_token = format!("csrf_{}", random_token());
        let created_at = now_secs();
        let expires_at = created_at + self.default_duration.as_secs_f64();
        let session = Session {
            id,
            user_id: user_id.to_string(),
            csrf_token,
            created_at,
            expires_at,
            ip_hash: None,
            user_agent_hash: None,
        };
        self.store.create(&session).await?;
        Ok(session)
    }

    /// Resolve a presented session id. Returns `Ok(None)` for a
    /// missing-or-expired session so callers can treat
    /// "not authenticated" uniformly. Expired rows are left in the table
    /// — the periodic [`SessionStore::purge_expired`] sweep removes them.
    pub async fn resolve(&self, id: &str) -> Result<Option<Session>> {
        let Some(session) = self.store.get(id).await? else {
            return Ok(None);
        };
        if session.expires_at <= now_secs() {
            return Ok(None);
        }
        Ok(Some(session))
    }

    /// Rotate a session id while preserving the user binding and the
    /// original `expires_at`. Used on privilege escalation
    /// (e.g. completing a step-up authentication) to defeat session
    /// fixation. Returns `Ok(None)` if the old id is unknown — caller
    /// can decide whether to surface as 401.
    pub async fn rotate(&self, old_id: &str) -> Result<Option<Session>> {
        let Some(old) = self.store.get(old_id).await? else {
            return Ok(None);
        };
        // Delete old before creating new so a crash in between can't
        // leave both sessions live for the same user with the same
        // expiry stamp.
        self.store.delete(old_id).await?;
        let new_id = format!("sess_{}", random_token());
        let csrf_token = format!("csrf_{}", random_token());
        let session = Session {
            id: new_id,
            user_id: old.user_id,
            csrf_token,
            created_at: now_secs(),
            expires_at: old.expires_at,
            ip_hash: old.ip_hash,
            user_agent_hash: old.user_agent_hash,
        };
        self.store.create(&session).await?;
        Ok(Some(session))
    }

    /// Revoke a single session — typically called from `/logout`.
    pub async fn revoke(&self, id: &str) -> Result<bool> {
        Ok(self.store.delete(id).await?)
    }

    /// Revoke every session for a user — typically called from
    /// "log out of all devices" or after a password change.
    pub async fn revoke_for_user(&self, user_id: &str) -> Result<u64> {
        Ok(self.store.delete_for_user(user_id).await?)
    }

    /// Borrow the underlying store. Phase 5/6 may want direct access
    /// (e.g. for the periodic purge sweep) without going through the
    /// manager's API surface.
    pub fn store(&self) -> &Arc<dyn SessionStore> {
        &self.store
    }
}

/// Build the HttpOnly session cookie that carries the opaque id.
///
/// `Secure; HttpOnly; SameSite=Lax; Path=/` matches the assumed
/// deployment shape — `public_url` only contributes its scheme today
/// (HTTPS in production), but is taken as a `Url` so the future
/// "domain= when running under a sub-domain" extension lands without an
/// API change.
pub fn cookie_for(session: &Session, public_url: &Url) -> Cookie<'static> {
    let max_age = max_age_for(session);
    let secure = is_secure(public_url);
    Cookie::build((SESSION_COOKIE, session.id.clone()))
        .path("/")
        .secure(secure)
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(max_age)
        .build()
        .into_owned()
}

/// Build the parallel CSRF cookie. Same path / max-age as the session
/// cookie but NOT HttpOnly so client JS can echo the value in a request
/// header on state-changing requests (double-submit pattern).
pub fn csrf_cookie_for(session: &Session) -> Cookie<'static> {
    let max_age = max_age_for(session);
    Cookie::build((CSRF_COOKIE, session.csrf_token.clone()))
        .path("/")
        .secure(true)
        .http_only(false)
        .same_site(SameSite::Lax)
        .max_age(max_age)
        .build()
        .into_owned()
}

/// Translate the session's wall-clock `expires_at` into a cookie
/// `Max-Age`. Clamped to `>= 0` so an already-expired session yields a
/// "delete me now" cookie instead of a negative age (some browsers
/// reject negative max-age outright).
fn max_age_for(session: &Session) -> CookieDuration {
    let secs = (session.expires_at - now_secs()).max(0.0) as i64;
    CookieDuration::seconds(secs)
}

/// HTTPS deployments (production) → Secure cookies. HTTP deployments
/// (local dev) → not Secure, otherwise the browser drops them. The
/// `public_url` is the operator-supplied canonical URL so this honours
/// the actual deployment, not the bind address (which may be 0.0.0.0
/// behind a TLS reverse proxy).
fn is_secure(public_url: &Url) -> bool {
    public_url.scheme().eq_ignore_ascii_case("https")
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

// =====================================================================
//   HTTP router — phase 8
// =====================================================================
//
// Endpoints:
// - POST  /login        password login: email + password → session cookie
// - DELETE /session     logout: revoke current session, clear cookie
// - GET   /whoami       current user's id + email
// - POST  /passkey/register/start   passkey ceremony — returns challenge
// - POST  /passkey/register/finish  finish — persists the cred via UserStore
// - POST  /passkey/auth/start
// - POST  /passkey/auth/finish

use axum::extract::{FromRef, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{delete, get, post};
use axum::Router;
use serde::Deserialize;
use serde_json::json;

use crate::ctx::AuthCtx;

/// Build the session router. Generic over a parent state `S` from
/// which `AuthCtx` is extractable via `axum::extract::FromRef`.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AuthCtx: FromRef<S>,
{
    Router::new()
        .route("/login", post(login_post))
        .route("/session", delete(logout_delete))
        .route("/whoami", get(whoami_get))
        .route("/passkey/register/start", post(passkey_register_start))
        .route("/passkey/register/finish", post(passkey_register_finish))
        .route("/passkey/auth/start", post(passkey_auth_start))
        .route("/passkey/auth/finish", post(passkey_auth_finish))
}

#[derive(Deserialize)]
struct LoginBody {
    email: String,
    password: String,
}

async fn login_post(State(ctx): State<AuthCtx>, Json(body): Json<LoginBody>) -> Response {
    let user = match ctx.users.get_user_by_email(&body.email).await {
        Ok(Some(u)) => u,
        _ => return unauthorized("invalid credentials"),
    };
    let stored = match ctx.users.get_password_hash(&user.id).await {
        Ok(Some(h)) => h,
        _ => return unauthorized("invalid credentials"),
    };
    let hasher = crate::password::PasswordHasher::default();
    let ok = match hasher.verify(&body.password, &stored) {
        Ok(b) => b,
        Err(_) => return unauthorized("invalid credentials"),
    };
    if !ok {
        return unauthorized("invalid credentials");
    }
    let mgr = SessionManager::with_default_duration(ctx.sessions.clone());
    let session = match mgr.create(&user.id).await {
        Ok(s) => s,
        Err(e) => return server_error(&format!("create session: {e}")),
    };
    let public_url = ctx
        .oidc_provider
        .as_ref()
        .map(|p| p.public_url.clone())
        .unwrap_or_else(|| url::Url::parse("http://localhost").unwrap());
    let cookie = cookie_for(&session, &public_url);
    let csrf = csrf_cookie_for(&session);
    let mut response = (
        StatusCode::OK,
        Json(json!({
            "user_id": user.id,
            "email": user.email,
            "csrf_token": session.csrf_token,
        })),
    )
        .into_response();
    if let Ok(value) = cookie.to_string().parse() {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    if let Ok(value) = csrf.to_string().parse() {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

async fn logout_delete(State(ctx): State<AuthCtx>, headers: HeaderMap) -> Response {
    if let Some(sid) = parse_cookie(&headers, SESSION_COOKIE) {
        let _ = ctx.sessions.delete(&sid).await;
    }
    let mut response = (StatusCode::NO_CONTENT, "").into_response();
    let clear = format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        SESSION_COOKIE
    );
    if let Ok(v) = clear.parse() {
        response.headers_mut().append(header::SET_COOKIE, v);
    }
    response
}

async fn whoami_get(State(ctx): State<AuthCtx>, headers: HeaderMap) -> Response {
    let sid = match parse_cookie(&headers, SESSION_COOKIE) {
        Some(s) => s,
        None => return unauthorized("no session"),
    };
    let mgr = SessionManager::with_default_duration(ctx.sessions.clone());
    let session = match mgr.resolve(&sid).await {
        Ok(Some(s)) => s,
        _ => return unauthorized("session unknown"),
    };
    let user = match ctx.users.get_user_by_id(&session.user_id).await {
        Ok(Some(u)) => u,
        _ => return unauthorized("user unknown"),
    };
    (
        StatusCode::OK,
        Json(json!({
            "user_id": user.id,
            "email": user.email,
            "email_verified": user.email_verified,
            "display_name": user.display_name,
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
struct PasskeyRegisterStartBody {
    user_id: String,
    user_name: String,
    display_name: String,
}

async fn passkey_register_start(
    State(ctx): State<AuthCtx>,
    Json(body): Json<PasskeyRegisterStartBody>,
) -> Response {
    let Some(mgr) = ctx.passkeys.as_ref() else {
        return svc_unavailable("passkey manager not configured");
    };
    let uuid = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, body.user_id.as_bytes());
    match mgr
        .start_registration(uuid, &body.user_name, &body.display_name, Some(&body.user_id))
        .await
    {
        Ok((challenge, state)) => {
            let state_blob = serde_json::to_string(&state).unwrap_or_default();
            (
                StatusCode::OK,
                Json(json!({
                    "challenge": challenge,
                    "state": state_blob,
                })),
            )
                .into_response()
        }
        Err(e) => bad_request(&format!("start_registration: {e}")),
    }
}

#[derive(Deserialize)]
struct PasskeyRegisterFinishBody {
    user_id: String,
    state: String,
    response: serde_json::Value,
}

async fn passkey_register_finish(
    State(ctx): State<AuthCtx>,
    Json(body): Json<PasskeyRegisterFinishBody>,
) -> Response {
    let Some(mgr) = ctx.passkeys.as_ref() else {
        return svc_unavailable("passkey manager not configured");
    };
    let state: webauthn_rs::prelude::PasskeyRegistration =
        match serde_json::from_str(&body.state) {
            Ok(s) => s,
            Err(e) => return bad_request(&format!("decode state: {e}")),
        };
    let response: webauthn_rs::prelude::RegisterPublicKeyCredential =
        match serde_json::from_value(body.response) {
            Ok(r) => r,
            Err(e) => return bad_request(&format!("decode response: {e}")),
        };
    let passkey = match mgr.finish_registration(&state, &response) {
        Ok(p) => p,
        Err(e) => return bad_request(&format!("finish_registration: {e}")),
    };
    let cred = crate::passkey::passkey_to_cred(&passkey, now_secs());
    if let Err(e) = ctx.users.add_passkey(&body.user_id, &cred).await {
        return server_error(&format!("persist passkey: {e}"));
    }
    (
        StatusCode::OK,
        Json(json!({"credential_id": data_encoding::BASE64URL_NOPAD.encode(&cred.credential_id)})),
    )
        .into_response()
}

#[derive(Deserialize)]
struct PasskeyAuthStartBody {
    user_id: String,
    /// Optional pre-decoded passkeys (for tests + advanced clients).
    /// In production we'd load these out of `auth.passkeys`; phase 8
    /// has no `passkey_json` column so we accept them as a body field.
    #[serde(default)]
    passkeys: Vec<serde_json::Value>,
}

async fn passkey_auth_start(
    State(ctx): State<AuthCtx>,
    Json(body): Json<PasskeyAuthStartBody>,
) -> Response {
    let Some(mgr) = ctx.passkeys.as_ref() else {
        return svc_unavailable("passkey manager not configured");
    };
    let _ = body.user_id;
    let mut creds: Vec<webauthn_rs::prelude::Passkey> = Vec::with_capacity(body.passkeys.len());
    for v in body.passkeys {
        match serde_json::from_value(v) {
            Ok(p) => creds.push(p),
            Err(e) => return bad_request(&format!("decode passkey: {e}")),
        }
    }
    if creds.is_empty() {
        return bad_request("passkeys list is empty");
    }
    match mgr.start_authentication_with(&creds) {
        Ok((challenge, state)) => (
            StatusCode::OK,
            Json(json!({
                "challenge": challenge,
                "state": serde_json::to_string(&state).unwrap_or_default(),
            })),
        )
            .into_response(),
        Err(e) => bad_request(&format!("start_authentication: {e}")),
    }
}

#[derive(Deserialize)]
struct PasskeyAuthFinishBody {
    state: String,
    response: serde_json::Value,
}

async fn passkey_auth_finish(
    State(ctx): State<AuthCtx>,
    Json(body): Json<PasskeyAuthFinishBody>,
) -> Response {
    let Some(mgr) = ctx.passkeys.as_ref() else {
        return svc_unavailable("passkey manager not configured");
    };
    let state: webauthn_rs::prelude::PasskeyAuthentication =
        match serde_json::from_str(&body.state) {
            Ok(s) => s,
            Err(e) => return bad_request(&format!("decode state: {e}")),
        };
    let response: webauthn_rs::prelude::PublicKeyCredential =
        match serde_json::from_value(body.response) {
            Ok(r) => r,
            Err(e) => return bad_request(&format!("decode response: {e}")),
        };
    match mgr.finish_authentication(&state, &response) {
        Ok(result) => (
            StatusCode::OK,
            Json(json!({
                "credential_id": data_encoding::BASE64URL_NOPAD.encode(&result.credential_id),
                "sign_count": result.sign_count,
                "user_verified": result.user_verified,
            })),
        )
            .into_response(),
        Err(e) => unauthorized(&format!("finish_authentication: {e}")),
    }
}

fn parse_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for kv in raw.split(';') {
        let kv = kv.trim();
        if let Some((k, v)) = kv.split_once('=') {
            if k == name {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn unauthorized(msg: &str) -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({"error": msg}))).into_response()
}
fn bad_request(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({"error": msg}))).into_response()
}
fn server_error(msg: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": msg})),
    )
        .into_response()
}
fn svc_unavailable(msg: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error": msg})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory store for fast unit tests. Mirrors the trait surface
    /// without touching sqlx.
    struct MemSessionStore(Mutex<HashMap<String, Session>>);

    impl MemSessionStore {
        fn new() -> Self {
            Self(Mutex::new(HashMap::new()))
        }
    }

    #[async_trait::async_trait]
    impl SessionStore for MemSessionStore {
        async fn create(&self, session: &Session) -> anyhow::Result<()> {
            self.0
                .lock()
                .unwrap()
                .insert(session.id.clone(), session.clone());
            Ok(())
        }

        async fn get(&self, id: &str) -> anyhow::Result<Option<Session>> {
            Ok(self.0.lock().unwrap().get(id).cloned())
        }

        async fn delete(&self, id: &str) -> anyhow::Result<bool> {
            Ok(self.0.lock().unwrap().remove(id).is_some())
        }

        async fn list_for_user(&self, user_id: &str) -> anyhow::Result<Vec<Session>> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .values()
                .filter(|s| s.user_id == user_id)
                .cloned()
                .collect())
        }

        async fn delete_for_user(&self, user_id: &str) -> anyhow::Result<u64> {
            let mut guard = self.0.lock().unwrap();
            let before = guard.len();
            guard.retain(|_, s| s.user_id != user_id);
            Ok((before - guard.len()) as u64)
        }

        async fn purge_expired(&self, now: f64) -> anyhow::Result<u64> {
            let mut guard = self.0.lock().unwrap();
            let before = guard.len();
            guard.retain(|_, s| s.expires_at > now);
            Ok((before - guard.len()) as u64)
        }

        async fn list_all(
            &self,
            limit: i64,
            offset: i64,
            user_filter: Option<&str>,
        ) -> anyhow::Result<Vec<Session>> {
            let guard = self.0.lock().unwrap();
            let mut all: Vec<Session> = guard
                .values()
                .filter(|s| user_filter.is_none_or(|u| s.user_id == u))
                .cloned()
                .collect();
            all.sort_by(|a, b| {
                b.created_at
                    .partial_cmp(&a.created_at)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let off = offset.max(0) as usize;
            let lim = limit.clamp(1, 500) as usize;
            Ok(all.into_iter().skip(off).take(lim).collect())
        }

        async fn count_all(&self, user_filter: Option<&str>) -> anyhow::Result<i64> {
            let guard = self.0.lock().unwrap();
            Ok(guard
                .values()
                .filter(|s| user_filter.is_none_or(|u| s.user_id == u))
                .count() as i64)
        }
    }

    fn manager() -> SessionManager {
        SessionManager::with_default_duration(Arc::new(MemSessionStore::new()))
    }

    #[tokio::test]
    async fn create_then_resolve_returns_same_session() {
        let mgr = manager();
        let created = mgr.create("user_alice").await.unwrap();
        let resolved = mgr.resolve(&created.id).await.unwrap().unwrap();
        assert_eq!(resolved.id, created.id);
        assert_eq!(resolved.user_id, "user_alice");
        assert!(created.id.starts_with("sess_"));
        assert!(created.csrf_token.starts_with("csrf_"));
    }

    #[tokio::test]
    async fn resolve_returns_none_for_unknown_id() {
        let mgr = manager();
        assert!(mgr.resolve("sess_nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn resolve_returns_none_for_expired_session() {
        // Build a session manually with expires_at in the past, then
        // probe through the manager.
        let store = Arc::new(MemSessionStore::new()) as Arc<dyn SessionStore>;
        let expired = Session {
            id: "sess_expired".to_string(),
            user_id: "user_x".to_string(),
            csrf_token: "csrf_expired".to_string(),
            created_at: now_secs() - 1000.0,
            expires_at: now_secs() - 1.0,
            ip_hash: None,
            user_agent_hash: None,
        };
        store.create(&expired).await.unwrap();
        let mgr = SessionManager::with_default_duration(store);
        assert!(mgr.resolve("sess_expired").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn rotate_returns_new_id_with_same_user_and_expiry() {
        let mgr = manager();
        let original = mgr.create("user_bob").await.unwrap();
        let rotated = mgr.rotate(&original.id).await.unwrap().unwrap();
        assert_ne!(rotated.id, original.id);
        assert_eq!(rotated.user_id, original.user_id);
        assert!((rotated.expires_at - original.expires_at).abs() < f64::EPSILON);
        // Old id is gone after rotation.
        assert!(mgr.resolve(&original.id).await.unwrap().is_none());
        // New id resolves.
        assert!(mgr.resolve(&rotated.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn revoke_drops_the_session() {
        let mgr = manager();
        let s = mgr.create("user_eve").await.unwrap();
        assert!(mgr.revoke(&s.id).await.unwrap());
        assert!(mgr.resolve(&s.id).await.unwrap().is_none());
        // Revoking again returns false (idempotent path).
        assert!(!mgr.revoke(&s.id).await.unwrap());
    }

    #[tokio::test]
    async fn revoke_for_user_drops_every_session_for_that_user() {
        let mgr = manager();
        let _s1 = mgr.create("user_multi").await.unwrap();
        let _s2 = mgr.create("user_multi").await.unwrap();
        let _other = mgr.create("user_other").await.unwrap();
        let dropped = mgr.revoke_for_user("user_multi").await.unwrap();
        assert_eq!(dropped, 2);
    }

    #[test]
    fn cookie_for_https_url_is_secure_httponly_lax() {
        let session = Session {
            id: "sess_abc".to_string(),
            user_id: "u".to_string(),
            csrf_token: "csrf_abc".to_string(),
            created_at: now_secs(),
            expires_at: now_secs() + 3600.0,
            ip_hash: None,
            user_agent_hash: None,
        };
        let url = Url::parse("https://app.example.com").unwrap();
        let cookie = cookie_for(&session, &url);
        assert_eq!(cookie.name(), SESSION_COOKIE);
        assert_eq!(cookie.value(), "sess_abc");
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.secure(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.path(), Some("/"));
    }

    #[test]
    fn csrf_cookie_is_not_http_only() {
        let session = Session {
            id: "sess_abc".to_string(),
            user_id: "u".to_string(),
            csrf_token: "csrf_abc".to_string(),
            created_at: now_secs(),
            expires_at: now_secs() + 3600.0,
            ip_hash: None,
            user_agent_hash: None,
        };
        let cookie = csrf_cookie_for(&session);
        assert_eq!(cookie.name(), CSRF_COOKIE);
        assert_eq!(cookie.value(), "csrf_abc");
        // CSRF token must be readable by client JS.
        assert_eq!(cookie.http_only(), Some(false));
    }

    #[test]
    fn cookie_for_http_url_is_not_secure() {
        let session = Session {
            id: "sess_abc".to_string(),
            user_id: "u".to_string(),
            csrf_token: "csrf_abc".to_string(),
            created_at: now_secs(),
            expires_at: now_secs() + 3600.0,
            ip_hash: None,
            user_agent_hash: None,
        };
        let url = Url::parse("http://localhost:3000").unwrap();
        let cookie = cookie_for(&session, &url);
        // Local-dev HTTP must not set Secure or the browser drops the cookie.
        assert_eq!(cookie.secure(), Some(false));
    }
}
