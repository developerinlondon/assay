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

// HTTP router — see `router()` below for the canonical route list.

use axum::Router;
use axum::extract::{FromRef, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{delete, get, post};
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
    mint_session_response(&ctx, &user.id, user.email.clone()).await
}

/// Mint an authenticated session for `user_id`, set the session + CSRF
/// cookies, and return the `200 OK` body. Shared by every successful
/// authentication path (password + passkey) so they log the user in
/// identically — same store, same cookies, same attributes.
async fn mint_session_response(
    ctx: &AuthCtx,
    user_id: &str,
    email: Option<String>,
) -> Response {
    let mgr = SessionManager::with_default_duration(ctx.sessions.clone());
    let session = match mgr.create(user_id).await {
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
            "user_id": user_id,
            "email": email,
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
        .start_registration(
            uuid,
            &body.user_name,
            &body.display_name,
            Some(&body.user_id),
        )
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
    let state: webauthn_rs::prelude::PasskeyRegistration = match serde_json::from_str(&body.state) {
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
    /// The user the client claims to be authenticating. We use it ONLY
    /// to look the user's registered credentials up in the server's
    /// store — the credential list itself is NEVER taken from the
    /// client. (A future discoverable-credential / userless flow can
    /// drop this field once `webauthn-rs` resident-key support is wired;
    /// until then the allowed-credentials list is server-resolved.)
    user_id: String,
}

async fn passkey_auth_start(
    State(ctx): State<AuthCtx>,
    Json(body): Json<PasskeyAuthStartBody>,
) -> Response {
    let Some(mgr) = ctx.passkeys.as_ref() else {
        return svc_unavailable("passkey manager not configured");
    };
    // Resolve the user's allowed credentials SERVER-SIDE from the store.
    // We never trust a client-supplied credential / passkey list — doing
    // so would let an attacker present a credential they control and
    // impersonate any account.
    let stored = match ctx.users.list_passkeys(&body.user_id).await {
        Ok(v) => v,
        Err(e) => return server_error(&format!("list passkeys: {e}")),
    };
    if stored.is_empty() {
        // Mirror the unauthorized shape so the endpoint doesn't leak
        // whether the user exists vs. simply has no passkeys.
        return unauthorized("no passkeys registered");
    }
    let mut creds: Vec<webauthn_rs::prelude::Passkey> = Vec::with_capacity(stored.len());
    for cred in &stored {
        match crate::passkey::cred_to_passkey(cred) {
            Ok(p) => creds.push(p),
            Err(e) => return server_error(&format!("rebuild stored passkey: {e}")),
        }
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
    let state: webauthn_rs::prelude::PasskeyAuthentication = match serde_json::from_str(&body.state)
    {
        Ok(s) => s,
        Err(e) => return bad_request(&format!("decode state: {e}")),
    };
    let response: webauthn_rs::prelude::PublicKeyCredential =
        match serde_json::from_value(body.response) {
            Ok(r) => r,
            Err(e) => return bad_request(&format!("decode response: {e}")),
        };
    // Verify the assertion. The library enforces the sign-counter here:
    // the in-progress `state` was built from the server-resolved stored
    // `Passkey` carrying its persisted counter, so a regression (clone /
    // replay) surfaces as an error and this fails closed.
    let result = match mgr.finish_authentication(&state, &response) {
        Ok(r) => r,
        Err(e) => return unauthorized(&format!("finish_authentication: {e}")),
    };

    // Resolve the owning user from the credential the authenticator
    // asserted — server-side, never from the client.
    let cred_id = result.cred_id().as_ref().to_vec();
    let (user_id, stored_cred) = match ctx.users.get_passkey(&cred_id).await {
        Ok(Some(pair)) => pair,
        Ok(None) => return unauthorized("unknown credential"),
        Err(e) => return server_error(&format!("get passkey: {e}")),
    };

    // Persist the sign-counter bump (+ backup-state) so the NEXT
    // authentication enforces against the new value. `update_credential`
    // returns the re-serialised blob; only write when it actually moved.
    let mut passkey = match crate::passkey::cred_to_passkey(&stored_cred) {
        Ok(p) => p,
        Err(e) => return server_error(&format!("rebuild stored passkey: {e}")),
    };
    if result.needs_update() {
        match crate::passkey::apply_auth_update(&mut passkey, &result) {
            Ok(blob) => {
                if let Err(e) = ctx
                    .users
                    .update_passkey_counter(&cred_id, result.counter(), &blob)
                    .await
                {
                    return server_error(&format!("persist sign counter: {e}"));
                }
            }
            Err(e) => return server_error(&format!("apply auth update: {e}")),
        }
    }

    // Mint a real authenticated session — identical to the password
    // login path. This is what actually logs the user in.
    let email = match ctx.users.get_user_by_id(&user_id).await {
        Ok(Some(u)) => u.email,
        _ => None,
    };
    mint_session_response(&ctx, &user_id, email).await
}

fn parse_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
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
    (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": msg}))).into_response()
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

    // ---- Passkey login security regressions (audit: session.rs) -------
    //
    // The three defects these cover:
    //   (a) trusted a client-supplied credential list
    //   (b) never enforced / persisted the sign counter
    //   (c) never minted a session on success
    //
    // A full browser ceremony needs an authenticator simulator; these
    // target the layers we own (request shape, store round-trip, session
    // minting + the server-side credential resolution path).

    use crate::ctx::AuthCtx;
    use crate::passkey::{PasskeyConfig, PasskeyManager};
    use crate::store::types::{PasskeyCred, User};
    use crate::store::UserStore;

    /// Minimal in-memory user store for the handler tests. Keyed by
    /// user_id; carries passkeys + a tiny user table.
    #[derive(Default)]
    struct MemUserStore {
        users: Mutex<HashMap<String, User>>,
        passkeys: Mutex<HashMap<String, Vec<PasskeyCred>>>,
    }

    #[async_trait::async_trait]
    impl UserStore for MemUserStore {
        async fn create_user(&self, user: &User) -> anyhow::Result<()> {
            self.users
                .lock()
                .unwrap()
                .insert(user.id.clone(), user.clone());
            Ok(())
        }
        async fn get_user_by_id(&self, id: &str) -> anyhow::Result<Option<User>> {
            Ok(self.users.lock().unwrap().get(id).cloned())
        }
        async fn get_user_by_email(&self, email: &str) -> anyhow::Result<Option<User>> {
            Ok(self
                .users
                .lock()
                .unwrap()
                .values()
                .find(|u| u.email.as_deref() == Some(email))
                .cloned())
        }
        async fn update_user(&self, _user: &User) -> anyhow::Result<()> {
            Ok(())
        }
        async fn list_users(
            &self,
            _limit: i64,
            _offset: i64,
            _search: Option<&str>,
        ) -> anyhow::Result<Vec<User>> {
            Ok(vec![])
        }
        async fn count_users(&self, _search: Option<&str>) -> anyhow::Result<i64> {
            Ok(0)
        }
        async fn delete_user(&self, _id: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
        async fn set_password_hash(&self, _user_id: &str, _hash: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn get_password_hash(&self, _user_id: &str) -> anyhow::Result<Option<String>> {
            Ok(None)
        }
        async fn list_passkeys(&self, user_id: &str) -> anyhow::Result<Vec<PasskeyCred>> {
            Ok(self
                .passkeys
                .lock()
                .unwrap()
                .get(user_id)
                .cloned()
                .unwrap_or_default())
        }
        async fn add_passkey(&self, user_id: &str, cred: &PasskeyCred) -> anyhow::Result<()> {
            self.passkeys
                .lock()
                .unwrap()
                .entry(user_id.to_string())
                .or_default()
                .push(cred.clone());
            Ok(())
        }
        async fn remove_passkey(&self, _credential_id: &[u8]) -> anyhow::Result<bool> {
            Ok(true)
        }
        async fn get_passkey(
            &self,
            credential_id: &[u8],
        ) -> anyhow::Result<Option<(String, PasskeyCred)>> {
            let guard = self.passkeys.lock().unwrap();
            for (uid, creds) in guard.iter() {
                if let Some(c) = creds.iter().find(|c| c.credential_id == credential_id) {
                    return Ok(Some((uid.clone(), c.clone())));
                }
            }
            Ok(None)
        }
        async fn update_passkey_counter(
            &self,
            credential_id: &[u8],
            sign_count: u32,
            passkey_json: &str,
        ) -> anyhow::Result<bool> {
            let mut guard = self.passkeys.lock().unwrap();
            for creds in guard.values_mut() {
                if let Some(c) = creds.iter_mut().find(|c| c.credential_id == credential_id) {
                    c.sign_count = sign_count;
                    c.passkey_json = Some(passkey_json.to_string());
                    return Ok(true);
                }
            }
            Ok(false)
        }
        async fn link_upstream(
            &self,
            _user_id: &str,
            _provider: &str,
            _subject: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn get_user_by_upstream(
            &self,
            _provider: &str,
            _subject: &str,
        ) -> anyhow::Result<Option<User>> {
            Ok(None)
        }
        async fn list_upstream_for_user(
            &self,
            _user_id: &str,
        ) -> anyhow::Result<Vec<(String, String)>> {
            Ok(vec![])
        }
    }

    fn test_passkey_manager(users: Arc<dyn UserStore>) -> PasskeyManager {
        let cfg = PasskeyConfig {
            rp_id: "localhost".to_string(),
            rp_name: "Assay Test".to_string(),
            origin: Url::parse("http://localhost:3000").unwrap(),
        };
        PasskeyManager::new(cfg, users).unwrap()
    }

    async fn body_json(resp: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    fn set_cookie_values(resp: &Response) -> Vec<String> {
        resp.headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
            .collect()
    }

    /// Defect (a): the request shape no longer carries a client-supplied
    /// credential list — a stray `passkeys` array is silently ignored, so
    /// a client can never inject the credential set the server matches
    /// against. (The allowed list is resolved server-side from the store.)
    #[test]
    fn auth_start_body_ignores_client_supplied_credential_list() {
        // Old attack shape: client tries to smuggle its own credentials.
        let raw = serde_json::json!({
            "user_id": "user_victim",
            "passkeys": [{"attacker": "controlled-credential"}]
        });
        let body: PasskeyAuthStartBody =
            serde_json::from_value(raw).expect("user_id-only body deserialises");
        assert_eq!(body.user_id, "user_victim");
        // The struct has exactly one field — there is nowhere for a
        // client credential list to land. This is enforced at the type
        // level: if someone re-adds a `passkeys` field this test's
        // companion (the handler test below) still proves the store is
        // the only credential source.
    }

    /// Defect (a), behavioural: `passkey_auth_start` resolves allowed
    /// credentials SERVER-SIDE. A user with no stored passkeys gets an
    /// unauthorized response — there is no client-supplied list that
    /// could substitute for the empty server store.
    #[tokio::test]
    async fn auth_start_uses_server_store_not_client_input() {
        let users: Arc<dyn UserStore> = Arc::new(MemUserStore::default());
        let sessions: Arc<dyn SessionStore> = Arc::new(MemSessionStore::new());
        let ctx = AuthCtx::new(users.clone(), sessions)
            .with_passkeys(test_passkey_manager(users));

        let resp = passkey_auth_start(
            State(ctx),
            Json(PasskeyAuthStartBody {
                user_id: "user_with_no_keys".to_string(),
            }),
        )
        .await;
        // No server-side credentials → fail closed, regardless of what a
        // client might have wanted to present.
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// Defect (b): the sign counter round-trips through the store. The
    /// stored `PasskeyCred` carries both the persisted `sign_count` and
    /// the serialised `Passkey` blob; `update_passkey_counter` bumps both
    /// and a re-read observes the new value. This is the persistence half
    /// of clone detection — without it the library could never compare
    /// against a moving counter.
    #[tokio::test]
    async fn sign_counter_persists_through_store_round_trip() {
        let users: Arc<dyn UserStore> = Arc::new(MemUserStore::default());
        let cred_id = vec![1u8, 2, 3, 4];
        let cred = PasskeyCred {
            credential_id: cred_id.clone(),
            public_key: vec![9, 9, 9],
            sign_count: 5,
            transports: vec![],
            created_at: now_secs(),
            passkey_json: Some("{\"blob\":\"v5\"}".to_string()),
        };
        users.add_passkey("user_a", &cred).await.unwrap();

        // Owner + blob resolve from the credential id alone (no client
        // identity trusted).
        let (owner, fetched) = users.get_passkey(&cred_id).await.unwrap().unwrap();
        assert_eq!(owner, "user_a");
        assert_eq!(fetched.sign_count, 5);
        assert!(fetched.passkey_json.is_some());

        // Persist a counter bump (what the finish handler does on a
        // legitimate, advancing authentication).
        let updated = users
            .update_passkey_counter(&cred_id, 6, "{\"blob\":\"v6\"}")
            .await
            .unwrap();
        assert!(updated);
        let (_, after) = users.get_passkey(&cred_id).await.unwrap().unwrap();
        assert_eq!(after.sign_count, 6);
        assert_eq!(after.passkey_json.as_deref(), Some("{\"blob\":\"v6\"}"));
    }

    /// Defect (b), library policy: a stored credential whose blob is
    /// missing (legacy row) fails closed on reconstruction rather than
    /// silently authenticating with a zero counter.
    #[test]
    fn legacy_credential_without_blob_fails_closed() {
        let legacy = PasskeyCred {
            credential_id: vec![7, 7],
            public_key: vec![1],
            sign_count: 0,
            transports: vec![],
            created_at: now_secs(),
            passkey_json: None,
        };
        let err = crate::passkey::cred_to_passkey(&legacy);
        assert!(err.is_err(), "legacy row must not yield a usable Passkey");
    }

    /// Defect (c): a successful authentication mints a REAL session —
    /// same store, same cookie pair, same attributes as the password
    /// path. We drive the shared minting helper the passkey finish
    /// handler uses and assert the session is resolvable and the cookies
    /// are set.
    #[tokio::test]
    async fn successful_auth_mints_a_real_resolvable_session() {
        let users: Arc<dyn UserStore> = Arc::new(MemUserStore::default());
        let store = Arc::new(MemSessionStore::new());
        let sessions: Arc<dyn SessionStore> = store.clone();
        users
            .create_user(&User {
                id: "user_real".to_string(),
                email: Some("real@example.com".to_string()),
                email_verified: true,
                display_name: None,
                created_at: now_secs(),
            })
            .await
            .unwrap();
        let ctx = AuthCtx::new(users, sessions.clone());

        let resp =
            mint_session_response(&ctx, "user_real", Some("real@example.com".to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);

        // Both cookies (session + CSRF) must be set.
        let cookies = set_cookie_values(&resp);
        assert!(
            cookies.iter().any(|c| c.starts_with(SESSION_COOKIE)),
            "session cookie missing: {cookies:?}"
        );
        assert!(
            cookies.iter().any(|c| c.starts_with(CSRF_COOKIE)),
            "csrf cookie missing: {cookies:?}"
        );

        // The session is actually persisted + resolvable for the user.
        let body = body_json(resp).await;
        assert_eq!(body["user_id"], "user_real");
        let listed = sessions.list_for_user("user_real").await.unwrap();
        assert_eq!(listed.len(), 1, "exactly one session minted");
        let mgr = SessionManager::with_default_duration(sessions);
        assert!(
            mgr.resolve(&listed[0].id).await.unwrap().is_some(),
            "minted session resolves"
        );
    }
}
