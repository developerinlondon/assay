//! Cross-cutting admin HTTP API.
//!
//! Phase 8b adds admin endpoints for user / session / Zanzibar / key
//! management. Each endpoint requires an admin api-key (compared in
//! constant time against [`crate::state::AdminApiKeys`]) — same auth
//! pattern as [`crate::oidc_provider::admin`].
//!
//! Surface (mounted under `/api/v1/engine/auth/` by the engine, so the
//! actual paths are `/api/v1/engine/auth/admin/...`):
//!
//! - `GET    /admin/users?limit=&offset=&search=`
//! - `POST   /admin/users`            → mint user
//! - `GET    /admin/users/{id}`       → user + linked passkeys + sessions + upstream
//! - `PUT    /admin/users/{id}`       → update email / display_name / verified
//! - `DELETE /admin/users/{id}`       → cascade delete via FKs
//! - `POST   /admin/users/{id}/password-reset` → set new password (admin override)
//!
//! - `GET    /admin/sessions?limit=&offset=&user_id=`
//! - `DELETE /admin/sessions/{id}`
//! - `DELETE /admin/sessions/by-user/{user_id}` → revoke all
//!
//! - `GET    /admin/biscuit`          → active root key info (kid + public PEM)
//! - `GET    /admin/jwks`             → JWKS public document (proxy /well-known)
//!
//! - `GET    /admin/zanzibar/namespaces`
//! - `POST   /admin/zanzibar/namespaces`           → define / replace schema
//! - `GET    /admin/zanzibar/namespaces/{name}`
//! - `POST   /admin/zanzibar/tuples`              → write
//! - `DELETE /admin/zanzibar/tuples`              → delete
//! - `POST   /admin/zanzibar/check`               → permission check
//! - `POST   /admin/zanzibar/expand`              → userset tree
//!
//! - `GET    /admin/audit?limit=&offset=&actor=&action=`
//!   → empty response today (audit table is deferred per V1 schema notes)

use axum::Router;
use axum::extract::{FromRef, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{delete, get, post};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ctx::AuthCtx;
use crate::state::AdminApiKeys;
use crate::store::User;

/// Build the cross-cutting admin router. Generic over a parent state
/// `S` from which `AuthCtx` and [`AdminApiKeys`] can be extracted via
/// `axum::extract::FromRef`.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AuthCtx: FromRef<S>,
    AdminApiKeys: FromRef<S>,
{
    Router::new()
        .route(
            "/admin/users",
            get(list_users).post(create_user_handler),
        )
        .route(
            "/admin/users/{id}",
            get(get_user_detail)
                .put(update_user_handler)
                .delete(delete_user_handler),
        )
        .route(
            "/admin/users/{id}/password-reset",
            post(password_reset_handler),
        )
        .route("/admin/sessions", get(list_sessions))
        .route("/admin/sessions/{id}", delete(revoke_session))
        .route(
            "/admin/sessions/by-user/{user_id}",
            delete(revoke_sessions_for_user),
        )
        .route("/admin/biscuit", get(biscuit_info))
        .route("/admin/jwks", get(jwks_proxy))
        .route(
            "/admin/zanzibar/namespaces",
            get(zanzibar_list_namespaces).post(zanzibar_define_namespace),
        )
        .route(
            "/admin/zanzibar/namespaces/{name}",
            get(zanzibar_get_namespace),
        )
        .route(
            "/admin/zanzibar/tuples",
            post(zanzibar_write_tuple).delete(zanzibar_delete_tuple),
        )
        .route("/admin/zanzibar/check", post(zanzibar_check_handler))
        .route("/admin/zanzibar/expand", post(zanzibar_expand_handler))
        .route("/admin/audit", get(audit_list))
}

/// Auth + Zanzibar gate shared by every admin handler. Resolves a
/// [`crate::gate::Caller`] from the request, then enforces the
/// `auth#system#admin` role. Admin api-key callers are accepted as
/// break-glass and bypass the Zanzibar lookup.
///
/// Returns the resolved caller on success — currently unused by these
/// handlers but kept so future audit-log integration can reach for it.
/// `Response` is ~272 bytes; the boxed error keeps the success path
/// small (every admin handler calls this on the hot path). Callers
/// unbox with `*r` before returning.
async fn require_admin(
    headers: &HeaderMap,
    ctx: &AuthCtx,
    keys: &AdminApiKeys,
) -> Result<crate::gate::Caller, Box<Response>> {
    crate::gate::require_role_for(headers, ctx, keys, "auth", "system", "admin").await
}

// =====================================================================
//   /admin/users
// =====================================================================

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ListUsersQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ListUsersResponse {
    pub items: Vec<User>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

async fn list_users(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Query(q): Query<ListUsersQuery>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let offset = q.offset.unwrap_or(0).max(0);
    let search = q.search.as_deref();
    let items = match ctx.users.list_users(limit, offset, search).await {
        Ok(v) => v,
        Err(e) => return server_error(&format!("list users: {e}")),
    };
    let total = match ctx.users.count_users(search).await {
        Ok(n) => n,
        Err(e) => return server_error(&format!("count users: {e}")),
    };
    (
        StatusCode::OK,
        Json(ListUsersResponse {
            items,
            total,
            limit,
            offset,
        }),
    )
        .into_response()
}

#[derive(Clone, Debug, Deserialize)]
pub struct CreateUserBody {
    pub email: Option<String>,
    pub display_name: Option<String>,
    #[serde(default)]
    pub email_verified: bool,
    /// Optional initial password. When present, hashed via Argon2id and
    /// stored on the user row. When `None`, the user has no password and
    /// must enrol a passkey or federate-in to authenticate.
    pub password: Option<String>,
}

async fn create_user_handler(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Json(body): Json<CreateUserBody>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    let id = format!(
        "usr_{}",
        data_encoding::BASE64URL_NOPAD.encode(&random_bytes::<12>())
    );
    let user = User {
        id: id.clone(),
        email: body.email,
        email_verified: body.email_verified,
        display_name: body.display_name,
        created_at: now_secs(),
    };
    if let Err(e) = ctx.users.create_user(&user).await {
        return server_error(&format!("create user: {e}"));
    }
    if let Some(pw) = body.password.as_ref() {
        #[cfg(feature = "auth-password")]
        {
            match crate::password::PasswordHasher::default().hash(pw) {
                Ok(hash) => {
                    if let Err(e) = ctx.users.set_password_hash(&id, &hash).await {
                        return server_error(&format!("set password hash: {e}"));
                    }
                }
                Err(e) => return server_error(&format!("hash password: {e}")),
            }
        }
        #[cfg(not(feature = "auth-password"))]
        {
            let _ = pw;
            return svc_unavailable("auth-password feature not compiled in");
        }
    }
    (StatusCode::CREATED, Json(user)).into_response()
}

#[derive(Clone, Debug, Serialize)]
pub struct UserDetailResponse {
    pub user: User,
    pub passkeys: Vec<PasskeySummary>,
    pub sessions: Vec<crate::store::Session>,
    pub upstream: Vec<UpstreamLink>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PasskeySummary {
    pub credential_id: String,
    pub sign_count: u32,
    pub transports: Vec<String>,
    pub created_at: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct UpstreamLink {
    pub provider: String,
    pub subject: String,
}

async fn get_user_detail(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    let user = match ctx.users.get_user_by_id(&id).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, Json(json!({"error": "unknown user_id"})))
                .into_response();
        }
        Err(e) => return server_error(&format!("get user: {e}")),
    };
    let passkeys = match ctx.users.list_passkeys(&id).await {
        Ok(v) => v
            .into_iter()
            .map(|p| PasskeySummary {
                credential_id: data_encoding::BASE64URL_NOPAD.encode(&p.credential_id),
                sign_count: p.sign_count,
                transports: p.transports,
                created_at: p.created_at,
            })
            .collect(),
        Err(e) => return server_error(&format!("list passkeys: {e}")),
    };
    let sessions = match ctx.sessions.list_for_user(&id).await {
        Ok(v) => v,
        Err(e) => return server_error(&format!("list sessions: {e}")),
    };
    let upstream = match ctx.users.list_upstream_for_user(&id).await {
        Ok(v) => v
            .into_iter()
            .map(|(provider, subject)| UpstreamLink { provider, subject })
            .collect(),
        Err(e) => return server_error(&format!("list upstream: {e}")),
    };
    (
        StatusCode::OK,
        Json(UserDetailResponse {
            user,
            passkeys,
            sessions,
            upstream,
        }),
    )
        .into_response()
}

#[derive(Clone, Debug, Deserialize)]
pub struct UpdateUserBody {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub email_verified: Option<bool>,
}

async fn update_user_handler(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<UpdateUserBody>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    let mut user = match ctx.users.get_user_by_id(&id).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, Json(json!({"error": "unknown user_id"})))
                .into_response();
        }
        Err(e) => return server_error(&format!("get user: {e}")),
    };
    if let Some(email) = body.email {
        user.email = Some(email);
    }
    if let Some(name) = body.display_name {
        user.display_name = Some(name);
    }
    if let Some(v) = body.email_verified {
        user.email_verified = v;
    }
    if let Err(e) = ctx.users.update_user(&user).await {
        return server_error(&format!("update user: {e}"));
    }
    (StatusCode::OK, Json(user)).into_response()
}

async fn delete_user_handler(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    match ctx.users.delete_user(&id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown user_id"})),
        )
            .into_response(),
        Err(e) => server_error(&format!("delete user: {e}")),
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct PasswordResetBody {
    /// New plaintext password — hashed by the handler via Argon2id and
    /// persisted on the user row. Admin-driven; bypasses the usual
    /// "old password" check that a self-service flow would enforce.
    pub password: String,
}

async fn password_reset_handler(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<PasswordResetBody>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    if ctx.users.get_user_by_id(&id).await.unwrap_or(None).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown user_id"})),
        )
            .into_response();
    }
    #[cfg(feature = "auth-password")]
    {
        let hash = match crate::password::PasswordHasher::default().hash(&body.password) {
            Ok(h) => h,
            Err(e) => return server_error(&format!("hash password: {e}")),
        };
        if let Err(e) = ctx.users.set_password_hash(&id, &hash).await {
            return server_error(&format!("set password hash: {e}"));
        }
        StatusCode::NO_CONTENT.into_response()
    }
    #[cfg(not(feature = "auth-password"))]
    {
        let _ = body.password;
        let _ = ctx;
        svc_unavailable("auth-password feature not compiled in")
    }
}

// =====================================================================
//   /admin/sessions
// =====================================================================

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ListSessionsQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub user_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ListSessionsResponse {
    pub items: Vec<crate::store::Session>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

async fn list_sessions(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Query(q): Query<ListSessionsQuery>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let offset = q.offset.unwrap_or(0).max(0);
    let user_filter = q.user_id.as_deref();
    let items = match ctx.sessions.list_all(limit, offset, user_filter).await {
        Ok(v) => v,
        Err(e) => return server_error(&format!("list sessions: {e}")),
    };
    let total = match ctx.sessions.count_all(user_filter).await {
        Ok(n) => n,
        Err(e) => return server_error(&format!("count sessions: {e}")),
    };
    (
        StatusCode::OK,
        Json(ListSessionsResponse {
            items,
            total,
            limit,
            offset,
        }),
    )
        .into_response()
}

async fn revoke_session(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    match ctx.sessions.delete(&id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown session_id"})),
        )
            .into_response(),
        Err(e) => server_error(&format!("revoke session: {e}")),
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RevokeAllResponse {
    pub revoked: u64,
}

async fn revoke_sessions_for_user(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    match ctx.sessions.delete_for_user(&user_id).await {
        Ok(n) => (StatusCode::OK, Json(RevokeAllResponse { revoked: n })).into_response(),
        Err(e) => server_error(&format!("revoke for user: {e}")),
    }
}

// =====================================================================
//   /admin/biscuit + /admin/jwks
// =====================================================================

#[derive(Clone, Debug, Serialize)]
pub struct BiscuitInfo {
    pub kid: String,
    pub public_pem: String,
}

async fn biscuit_info(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    let kid = ctx.biscuit.active_kid();
    let public_pem = match ctx.biscuit.public_pem() {
        Ok(s) => s,
        Err(e) => return server_error(&format!("public pem: {e}")),
    };
    (StatusCode::OK, Json(BiscuitInfo { kid, public_pem })).into_response()
}

async fn jwks_proxy(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    // The OIDC provider's JWKS endpoint already enumerates the active
    // signing key — reuse its shape so admin tooling sees the same
    // payload non-admin discovery would. When the OIDC provider isn't
    // wired, return an empty key set.
    #[cfg(feature = "auth-oidc-provider")]
    {
        if let Some(provider) = ctx.oidc_provider.as_ref() {
            let payload = match &provider.jwks_source {
                #[cfg(feature = "backend-postgres")]
                crate::oidc_provider::JwksSource::Postgres(_) => json!({"keys": []}),
                #[cfg(feature = "backend-sqlite")]
                crate::oidc_provider::JwksSource::Sqlite(_) => json!({"keys": []}),
                crate::oidc_provider::JwksSource::Memory(v) => json!({"keys": v}),
            };
            return (StatusCode::OK, Json(payload)).into_response();
        }
    }
    let _ = ctx;
    (StatusCode::OK, Json(json!({"keys": []}))).into_response()
}

// =====================================================================
//   /admin/zanzibar
// =====================================================================

async fn zanzibar_list_namespaces(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    #[cfg(feature = "auth-zanzibar")]
    {
        let Some(store) = ctx.zanzibar.as_ref() else {
            return svc_unavailable("zanzibar not enabled");
        };
        return match store.list_namespaces().await {
            Ok(v) => (StatusCode::OK, Json(v)).into_response(),
            Err(e) => server_error(&format!("list namespaces: {e}")),
        };
    }
    #[cfg(not(feature = "auth-zanzibar"))]
    {
        let _ = ctx;
        svc_unavailable("zanzibar not compiled in")
    }
}

async fn zanzibar_get_namespace(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    #[cfg(feature = "auth-zanzibar")]
    {
        let Some(store) = ctx.zanzibar.as_ref() else {
            return svc_unavailable("zanzibar not enabled");
        };
        return match store.get_namespace(&name).await {
            Ok(Some(ns)) => (StatusCode::OK, Json(ns)).into_response(),
            Ok(None) => (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "unknown namespace"})),
            )
                .into_response(),
            Err(e) => server_error(&format!("get namespace: {e}")),
        };
    }
    #[cfg(not(feature = "auth-zanzibar"))]
    {
        let _ = (ctx, name);
        svc_unavailable("zanzibar not compiled in")
    }
}

async fn zanzibar_define_namespace(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Json(schema): Json<crate::zanzibar::NamespaceSchema>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    #[cfg(feature = "auth-zanzibar")]
    {
        let Some(store) = ctx.zanzibar.as_ref() else {
            return svc_unavailable("zanzibar not enabled");
        };
        return match store.define_namespace(&schema).await {
            Ok(()) => (
                StatusCode::CREATED,
                Json(json!({"ok": true, "name": schema.name})),
            )
                .into_response(),
            Err(e) => server_error(&format!("define namespace: {e}")),
        };
    }
    #[cfg(not(feature = "auth-zanzibar"))]
    {
        let _ = (ctx, schema);
        svc_unavailable("zanzibar not compiled in")
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct TupleBody {
    pub object_type: String,
    pub object_id: String,
    pub relation: String,
    pub subject_type: String,
    pub subject_id: String,
    #[serde(default)]
    pub subject_rel: Option<String>,
}

async fn zanzibar_write_tuple(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Json(body): Json<TupleBody>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    #[cfg(feature = "auth-zanzibar")]
    {
        let Some(store) = ctx.zanzibar.as_ref() else {
            return svc_unavailable("zanzibar not enabled");
        };
        let tuple = body_to_tuple(body);
        return match store.write_tuple(&tuple).await {
            Ok(()) => (StatusCode::CREATED, Json(json!({"ok": true}))).into_response(),
            Err(e) => server_error(&format!("write tuple: {e}")),
        };
    }
    #[cfg(not(feature = "auth-zanzibar"))]
    {
        let _ = (ctx, body);
        svc_unavailable("zanzibar not compiled in")
    }
}

async fn zanzibar_delete_tuple(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Json(body): Json<TupleBody>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    #[cfg(feature = "auth-zanzibar")]
    {
        let Some(store) = ctx.zanzibar.as_ref() else {
            return svc_unavailable("zanzibar not enabled");
        };
        let tuple = body_to_tuple(body);
        return match store.delete_tuple(&tuple).await {
            Ok(true) => StatusCode::NO_CONTENT.into_response(),
            Ok(false) => (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "tuple not found"})),
            )
                .into_response(),
            Err(e) => server_error(&format!("delete tuple: {e}")),
        };
    }
    #[cfg(not(feature = "auth-zanzibar"))]
    {
        let _ = (ctx, body);
        svc_unavailable("zanzibar not compiled in")
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct CheckBody {
    pub resource_type: String,
    pub resource_id: String,
    pub permission: String,
    pub subject_type: String,
    pub subject_id: String,
    #[serde(default)]
    pub subject_rel: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CheckResponse {
    pub result: String,
    pub allowed: bool,
}

async fn zanzibar_check_handler(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Json(body): Json<CheckBody>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    #[cfg(feature = "auth-zanzibar")]
    {
        use crate::zanzibar::{CheckResult, Consistency, ObjectRef, SubjectRef};
        let Some(store) = ctx.zanzibar.as_ref() else {
            return svc_unavailable("zanzibar not enabled");
        };
        let resource = ObjectRef {
            object_type: body.resource_type,
            object_id: body.resource_id,
        };
        let subject = SubjectRef {
            subject_type: body.subject_type,
            subject_id: body.subject_id,
            subject_rel: body.subject_rel,
        };
        return match store
            .check(&resource, &body.permission, &subject, Consistency::Minimum)
            .await
        {
            Ok(r) => {
                let (label, allowed) = match &r {
                    CheckResult::Allowed { .. } => ("Allowed", true),
                    CheckResult::Denied => ("Denied", false),
                    CheckResult::DepthExceeded => ("DepthExceeded", false),
                    CheckResult::CycleDetected => ("CycleDetected", false),
                };
                (
                    StatusCode::OK,
                    Json(CheckResponse {
                        result: label.to_string(),
                        allowed,
                    }),
                )
                    .into_response()
            }
            Err(e) => server_error(&format!("check: {e}")),
        };
    }
    #[cfg(not(feature = "auth-zanzibar"))]
    {
        let _ = (ctx, body);
        svc_unavailable("zanzibar not compiled in")
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ExpandBody {
    pub resource_type: String,
    pub resource_id: String,
    pub relation: String,
    #[serde(default)]
    pub depth_limit: Option<u32>,
}

async fn zanzibar_expand_handler(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Json(body): Json<ExpandBody>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    #[cfg(feature = "auth-zanzibar")]
    {
        use crate::zanzibar::{ObjectRef, MAX_DEPTH};
        let Some(store) = ctx.zanzibar.as_ref() else {
            return svc_unavailable("zanzibar not enabled");
        };
        let resource = ObjectRef {
            object_type: body.resource_type,
            object_id: body.resource_id,
        };
        let depth = body.depth_limit.unwrap_or(MAX_DEPTH);
        return match store.expand(&resource, &body.relation, depth).await {
            Ok(tree) => (StatusCode::OK, Json(tree)).into_response(),
            Err(e) => server_error(&format!("expand: {e}")),
        };
    }
    #[cfg(not(feature = "auth-zanzibar"))]
    {
        let _ = (ctx, body);
        svc_unavailable("zanzibar not compiled in")
    }
}

// =====================================================================
//   /admin/audit
// =====================================================================

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ListAuditQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub since: Option<f64>,
    #[serde(default)]
    pub until: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AuditResponse {
    pub items: Vec<serde_json::Value>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
    /// `false` until the `auth.audit` table is materialised (see
    /// `crate::schema` notes — deferred to a later phase). The
    /// dashboard renders an empty-state with this value to explain
    /// the missing rows.
    pub enabled: bool,
}

async fn audit_list(
    State(ctx): State<AuthCtx>,
    State(keys): State<AdminApiKeys>,
    headers: HeaderMap,
    Query(q): Query<ListAuditQuery>,
) -> Response {
    if let Err(r) = require_admin(&headers, &ctx, &keys).await {
        return *r;
    }
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let offset = q.offset.unwrap_or(0).max(0);
    (
        StatusCode::OK,
        Json(AuditResponse {
            items: Vec::new(),
            total: 0,
            limit,
            offset,
            enabled: false,
        }),
    )
        .into_response()
}

// =====================================================================
//   helpers
// =====================================================================

#[cfg(feature = "auth-zanzibar")]
fn body_to_tuple(body: TupleBody) -> crate::zanzibar::Tuple {
    crate::zanzibar::Tuple {
        object_type: body.object_type,
        object_id: body.object_id,
        relation: body.relation,
        subject_type: body.subject_type,
        subject_id: body.subject_id,
        subject_rel: body.subject_rel,
    }
}

fn server_error(msg: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "server_error", "error_description": msg})),
    )
        .into_response()
}

fn svc_unavailable(msg: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error": "service_unavailable", "error_description": msg})),
    )
        .into_response()
}

fn now_secs() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn random_bytes<const N: usize>() -> [u8; N] {
    use rand::RngCore;
    let mut buf = [0u8; N];
    rand::rng().fill_bytes(&mut buf);
    buf
}

// Admin-gate behaviour is covered in `crate::gate::tests` and the
// integration-test suite — the local `require_admin` is now a
// one-line wrapper, so a per-handler test would duplicate gate.rs's
// coverage without exercising new code paths.
