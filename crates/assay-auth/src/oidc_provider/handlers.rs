//! Concrete axum handlers for the OIDC provider.
//!
//! Phase 8 wiring — every helper module (authorize / token / userinfo /
//! revoke / introspect / federation) ships pure validation/build logic;
//! this file glues them to `axum::extract::State<AuthCtx>` so the
//! engine binary's router can serve real HTTP responses.
//!
//! Conventions:
//!
//! - Every handler returns an `axum::response::Response` so we can mix
//!   redirects, JSON bodies, and HTML pages without bespoke wrappers.
//! - Errors map to `(StatusCode, Json<...>)` tuples shaped like the
//!   relevant RFC's error body (OIDC Core, OAuth 2 §5.2, RFC 7009/7662).
//! - Session cookies are read off the `Cookie` header by name; we don't
//!   use `axum-extra::CookieJar` to keep the dep set lean.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::Form;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Json, Redirect, Response};
use serde::Deserialize;
use serde_json::json;

use crate::ctx::AuthCtx;

use super::authorize::{self as authz, AuthorizeRequest, AuthorizeValidation};
use super::consent::{ConsentPage, ConsentSubmission, scopes_already_granted};
use super::introspect::{IntrospectRequest, IntrospectResponse};
use super::revoke::RevokeRequest;
use super::token::{self as tok, TokenErrorBody, TokenRequest, TokenResponse, errors};
use super::types::{ConsentGrant, OidcSession};
use super::userinfo::{self, AccessTokenClaims};

/// Cookie name carrying a transient "in-flight authorize request"
/// payload. We base64-url-encode the original querystring so the
/// consent POST can resume without persisting state.
const RESUME_COOKIE: &str = "assay_oidc_resume";

/// Cookie name carrying the raw upstream-OIDC binding token. Set on
/// `/oidc/upstream/{slug}/start`'s 302; checked by `/callback` against
/// the state row's `binding_hash`. Cleared on every callback response
/// (success or failure).
const UPSTREAM_BINDING_COOKIE: &str = "assay_oidc_binding";

// =====================================================================
//   /authorize
// =====================================================================

/// `GET /authorize` — orchestrates: validate → check session → render
/// consent (or skip on prior grant) → mint code → 302 to the consumer.
pub async fn authorize_get(
    State(ctx): State<AuthCtx>,
    headers: HeaderMap,
    Query(req): Query<AuthorizeRequest>,
) -> Response {
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => return server_misconfigured("oidc_provider is not enabled"),
    };

    // Look up the registered client.
    let client = match provider.clients.get(&req.client_id).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return error_html(
                StatusCode::BAD_REQUEST,
                &format!("unknown client_id {:?}", req.client_id),
            );
        }
        Err(e) => return server_error_html(&format!("client lookup failed: {e}")),
    };

    // Pure validation against the registered client.
    match authz::validate(&req, &client) {
        AuthorizeValidation::Ok { scopes } => {
            authorize_post_validate(ctx.clone(), &headers, req, client, scopes).await
        }
        AuthorizeValidation::Fatal { reason } => error_html(StatusCode::BAD_REQUEST, &reason),
        AuthorizeValidation::Redirect { error, description } => {
            Redirect::to(&authz::redirect_with_error(
                &req.redirect_uri,
                error,
                &description,
                req.state.as_deref(),
            ))
            .into_response()
        }
    }
}

/// Branch of [`authorize_get`] for the validated path. Resolves the
/// session cookie; if no live session, redirects to `/auth/login`. If
/// authenticated, mints either the consent page or the code directly.
async fn authorize_post_validate(
    ctx: AuthCtx,
    headers: &HeaderMap,
    req: AuthorizeRequest,
    client: super::types::OidcClient,
    scopes: Vec<String>,
) -> Response {
    // Resolve the session cookie.
    let session_id = parse_cookie(headers, crate::session::SESSION_COOKIE);
    let session = match session_id {
        Some(sid) => match ctx.sessions.get(&sid).await {
            Ok(Some(s)) if s.expires_at > now_secs() => Some(s),
            _ => None,
        },
        None => None,
    };

    // No session → redirect to login with the original URL stashed.
    let Some(session) = session else {
        let original = rebuild_authorize_url(&ctx, &req);
        return Redirect::to(&authz::return_to_for(&original)).into_response();
    };

    // Authenticated. Decide whether consent is required.
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => return server_misconfigured("oidc_provider is not enabled"),
    };

    let needs_consent = if !client.require_consent {
        false
    } else {
        // Skip consent when the user has previously granted these (or
        // wider) scopes for this client.
        match provider
            .consents
            .get(&session.user_id, &client.client_id)
            .await
        {
            Ok(Some(grant)) => !scopes_already_granted(&scopes, &grant.scopes),
            _ => true,
        }
    };

    if needs_consent {
        let resume = encode_resume(&req);
        let page = ConsentPage {
            client_name: &client.name,
            issuer: &provider.issuer,
            scopes: &scopes,
            csrf_token: &session.csrf_token,
            resume_token: &resume,
        };
        let mut response = Html(page.render_html()).into_response();
        // Set the resume cookie so the consent POST can rebuild the
        // authorize request without trusting form payload alone.
        if let Ok(value) = format!(
            "{}={}; Path=/; HttpOnly; SameSite=Lax",
            RESUME_COOKIE, resume
        )
        .parse()
        {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
        return response;
    }

    // No consent required — issue the code.
    issue_authorization_code(&ctx, req, &session.user_id, scopes).await
}

/// Common path: build + persist an [`AuthorizationCode`] row, then
/// 302 the user back to the consumer's `redirect_uri`.
async fn issue_authorization_code(
    ctx: &AuthCtx,
    req: AuthorizeRequest,
    user_id: &str,
    scopes: Vec<String>,
) -> Response {
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => return server_misconfigured("oidc_provider is not enabled"),
    };
    let code = authz::build_code(user_id, &req, scopes);
    if let Err(e) = provider.codes.create(&code).await {
        return server_error_html(&format!("persist authorization code: {e}"));
    }
    let redirect = authz::redirect_with_code(&req.redirect_uri, &code.code, req.state.as_deref());
    Redirect::to(&redirect).into_response()
}

/// Reconstruct the original `/authorize` URL so the post-login flow
/// can resume. We use the OIDC provider's issuer as the base.
fn rebuild_authorize_url(ctx: &AuthCtx, req: &AuthorizeRequest) -> String {
    let issuer = ctx
        .oidc_provider
        .as_ref()
        .map(|p| p.issuer.as_str())
        .unwrap_or("");
    let mut url = format!(
        "{issuer}/authorize?response_type={}",
        url_encode(&req.response_type)
    );
    url.push_str(&format!("&client_id={}", url_encode(&req.client_id)));
    url.push_str(&format!("&redirect_uri={}", url_encode(&req.redirect_uri)));
    url.push_str(&format!("&scope={}", url_encode(&req.scope)));
    if let Some(s) = &req.state {
        url.push_str(&format!("&state={}", url_encode(s)));
    }
    if let Some(n) = &req.nonce {
        url.push_str(&format!("&nonce={}", url_encode(n)));
    }
    if let Some(c) = &req.code_challenge {
        url.push_str(&format!("&code_challenge={}", url_encode(c)));
    }
    if let Some(m) = &req.code_challenge_method {
        url.push_str(&format!("&code_challenge_method={}", url_encode(m)));
    }
    url
}

// =====================================================================
//   /authorize/consent
// =====================================================================

/// `POST /authorize/consent` — user clicked Allow / Deny on the consent
/// page. We rebuild the original `AuthorizeRequest` from the resume
/// cookie, persist the consent (on Allow), then issue the code or the
/// `error=access_denied` redirect.
pub async fn consent_post(
    State(ctx): State<AuthCtx>,
    headers: HeaderMap,
    Form(submission): Form<ConsentSubmission>,
) -> Response {
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => return server_misconfigured("oidc_provider is not enabled"),
    };

    // Pull the resume payload from the cookie (defended against form
    // tampering by anchoring on the cookie, not the form field).
    let resume = match parse_cookie(&headers, RESUME_COOKIE) {
        Some(c) => c,
        None => {
            return error_html(
                StatusCode::BAD_REQUEST,
                "consent flow has no resume token (cookie missing)",
            );
        }
    };
    let req = match decode_resume(&resume) {
        Some(r) => r,
        None => {
            return error_html(
                StatusCode::BAD_REQUEST,
                "consent resume payload is malformed",
            );
        }
    };

    // CSRF: token must match the session's stored csrf_token.
    let session = match parse_cookie(&headers, crate::session::SESSION_COOKIE) {
        Some(sid) => ctx.sessions.get(&sid).await.ok().flatten(),
        None => None,
    };
    let Some(session) = session else {
        return error_html(StatusCode::UNAUTHORIZED, "no active session");
    };
    if session.csrf_token != submission.csrf_token {
        return error_html(StatusCode::FORBIDDEN, "csrf mismatch");
    }

    // Look up the client + revalidate (it might have been deleted in
    // the millisecond between authorize_get and now).
    let client = match provider.clients.get(&req.client_id).await {
        Ok(Some(c)) => c,
        _ => return error_html(StatusCode::BAD_REQUEST, "unknown client_id"),
    };
    let scopes: Vec<String> = req
        .scope
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    if !submission.allowed() {
        // Deny → redirect with error=access_denied.
        let redirect = authz::redirect_with_error(
            &req.redirect_uri,
            "access_denied",
            "user denied consent",
            req.state.as_deref(),
        );
        return Redirect::to(&redirect).into_response();
    }

    // Persist the consent grant.
    let grant = ConsentGrant {
        user_id: session.user_id.clone(),
        client_id: client.client_id.clone(),
        scopes: scopes.clone(),
        granted_at: now_secs(),
    };
    if let Err(e) = provider.consents.upsert(&grant).await {
        return server_error_html(&format!("persist consent: {e}"));
    }

    issue_authorization_code(&ctx, req, &session.user_id, scopes).await
}

// =====================================================================
//   /token
// =====================================================================

/// `POST /token` — dispatch to `authorization_code` or `refresh_token`.
/// `client_credentials` is reserved (returns `unsupported_grant_type`).
pub async fn token_post(
    State(ctx): State<AuthCtx>,
    headers: HeaderMap,
    Form(req): Form<TokenRequest>,
) -> Response {
    let _provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => return server_misconfigured("oidc_provider is not enabled"),
    };

    // Authenticate the client (basic / post / none-PKCE).
    let client = match authenticate_client(&ctx, &headers, &req).await {
        Ok(c) => c,
        Err((status, body)) => return (status, Json(body)).into_response(),
    };

    match req.grant_type.as_str() {
        "authorization_code" => grant_authorization_code(&ctx, &client, &req).await,
        "refresh_token" => grant_refresh(&ctx, &client, &req).await,
        other => token_err(
            StatusCode::BAD_REQUEST,
            errors::UNSUPPORTED_GRANT_TYPE,
            Some(format!("grant_type {other:?} is not supported")),
        ),
    }
}

/// Authenticate the client — supports `client_secret_basic`,
/// `client_secret_post`, or PKCE-only `none`. Returns either the loaded
/// client row or a wire-shaped error tuple.
async fn authenticate_client(
    ctx: &AuthCtx,
    headers: &HeaderMap,
    req: &TokenRequest,
) -> Result<super::types::OidcClient, (StatusCode, TokenErrorBody)> {
    let provider = ctx.oidc_provider.as_ref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            err_body(errors::SERVER_ERROR, None),
        )
    })?;

    // Basic header: "Basic base64(client_id:client_secret)".
    let basic = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Basic "))
        .or_else(|| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("basic "))
        })
        .and_then(|enc| data_encoding::BASE64.decode(enc.as_bytes()).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|s| {
            let (id, secret) = s.split_once(':')?;
            Some((id.to_string(), secret.to_string()))
        });

    let (client_id, presented_secret) = match (basic, &req.client_id) {
        (Some((id, secret)), _) => (id, Some(secret)),
        (None, Some(id)) => (id.clone(), req.client_secret.clone()),
        (None, None) => {
            return Err((
                StatusCode::UNAUTHORIZED,
                err_body(errors::INVALID_CLIENT, Some("client_id missing".into())),
            ));
        }
    };

    let client = match provider.clients.get(&client_id).await {
        Ok(Some(c)) => c,
        _ => {
            return Err((
                StatusCode::UNAUTHORIZED,
                err_body(errors::INVALID_CLIENT, Some("unknown client".into())),
            ));
        }
    };

    match client.token_endpoint_auth_method {
        super::types::TokenAuthMethod::None => {
            // Public PKCE-only client — no shared secret.
            Ok(client)
        }
        super::types::TokenAuthMethod::ClientSecretBasic
        | super::types::TokenAuthMethod::ClientSecretPost => {
            let presented = presented_secret
                .as_deref()
                .map(|s| s.to_string())
                .unwrap_or_default();
            let stored = client.client_secret_hash.as_deref().unwrap_or("");
            if !verify_client_secret(&presented, stored) {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    err_body(errors::INVALID_CLIENT, Some("bad secret".into())),
                ));
            }
            Ok(client)
        }
        super::types::TokenAuthMethod::PrivateKeyJwt => {
            // Reserved for v0.2.0+; reject for now.
            Err((
                StatusCode::BAD_REQUEST,
                err_body(
                    errors::INVALID_CLIENT,
                    Some("private_key_jwt not yet supported".into()),
                ),
            ))
        }
    }
}

/// Constant-time secret check. The stored hash is either an Argon2 PHC
/// string (`$argon2id$...`) or — for the simpler v0.2.0 surface — the
/// plaintext secret. We try Argon2 first and fall back to bytewise
/// compare so the migration path to PHC-only doesn't require a flag-day.
fn verify_client_secret(presented: &str, stored: &str) -> bool {
    if stored.starts_with("$argon2") {
        let hasher = crate::password::PasswordHasher::default();
        return hasher.verify(presented, stored).unwrap_or(false);
    }
    // Plaintext fallback — constant-time bytewise compare.
    let a = presented.as_bytes();
    let b = stored.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// `authorization_code` grant — consume the code, verify PKCE, mint
/// id_token + access_token (+ optionally refresh_token), record the
/// SSO session row.
async fn grant_authorization_code(
    ctx: &AuthCtx,
    client: &super::types::OidcClient,
    req: &TokenRequest,
) -> Response {
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => {
            return token_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::SERVER_ERROR,
                None,
            );
        }
    };
    let Some(code_str) = req.code.as_deref() else {
        return token_err(
            StatusCode::BAD_REQUEST,
            errors::INVALID_REQUEST,
            Some("code is required".into()),
        );
    };
    let consumed = match provider.codes.consume(code_str).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return token_err(
                StatusCode::BAD_REQUEST,
                errors::INVALID_GRANT,
                Some("code is unknown or already used".into()),
            );
        }
        Err(e) => {
            return token_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::SERVER_ERROR,
                Some(format!("consume code: {e}")),
            );
        }
    };
    if consumed.expires_at <= now_secs() {
        return token_err(
            StatusCode::BAD_REQUEST,
            errors::INVALID_GRANT,
            Some("code expired".into()),
        );
    }
    if consumed.client_id != client.client_id {
        return token_err(
            StatusCode::BAD_REQUEST,
            errors::INVALID_GRANT,
            Some("code does not belong to this client".into()),
        );
    }
    if let Some(redirect) = &req.redirect_uri
        && redirect != &consumed.redirect_uri
    {
        return token_err(
            StatusCode::BAD_REQUEST,
            errors::INVALID_GRANT,
            Some("redirect_uri mismatch".into()),
        );
    }
    // PKCE verify (S256 only — challenge_method always normalised).
    if !consumed.code_challenge.is_empty() {
        let verifier = req.code_verifier.as_deref().unwrap_or("");
        if !tok::verify_pkce_s256(verifier, &consumed.code_challenge) {
            return token_err(
                StatusCode::BAD_REQUEST,
                errors::INVALID_GRANT,
                Some("PKCE verifier mismatch".into()),
            );
        }
    }

    issue_token_pair(
        ctx,
        client,
        &consumed.user_id,
        &consumed.scopes,
        consumed.nonce.as_deref(),
    )
    .await
}

/// `refresh_token` grant — verify, rotate (revoke old, mint new), mint
/// fresh access + id token. Replay → revoke every refresh token for the
/// user (OAuth 2.1 replay-detection nuke).
async fn grant_refresh(
    ctx: &AuthCtx,
    client: &super::types::OidcClient,
    req: &TokenRequest,
) -> Response {
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => {
            return token_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::SERVER_ERROR,
                None,
            );
        }
    };
    let Some(presented) = req.refresh_token.as_deref() else {
        return token_err(
            StatusCode::BAD_REQUEST,
            errors::INVALID_REQUEST,
            Some("refresh_token is required".into()),
        );
    };
    let hash = tok::hash_refresh_token(presented);
    let row = match provider.refresh.get(&hash).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return token_err(
                StatusCode::BAD_REQUEST,
                errors::INVALID_GRANT,
                Some("refresh_token unknown".into()),
            );
        }
        Err(e) => {
            return token_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::SERVER_ERROR,
                Some(format!("refresh lookup: {e}")),
            );
        }
    };
    if row.revoked {
        // Replay detected — revoke every token belonging to this user.
        let _ = provider.refresh.revoke_for_user(&row.user_id).await;
        return token_err(
            StatusCode::BAD_REQUEST,
            errors::INVALID_GRANT,
            Some("refresh_token revoked (replay detected)".into()),
        );
    }
    if row.expires_at <= now_secs() {
        return token_err(
            StatusCode::BAD_REQUEST,
            errors::INVALID_GRANT,
            Some("refresh_token expired".into()),
        );
    }
    if row.client_id != client.client_id {
        return token_err(
            StatusCode::BAD_REQUEST,
            errors::INVALID_GRANT,
            Some("refresh_token client mismatch".into()),
        );
    }
    if let Err(e) = provider.refresh.revoke(&hash).await {
        return token_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            errors::SERVER_ERROR,
            Some(format!("revoke old refresh: {e}")),
        );
    }
    issue_token_pair(ctx, client, &row.user_id, &row.scopes, None).await
}

/// Mint id_token + access_token + refresh_token (when `offline_access`
/// or refresh-token grant in the client's allow-list) and record the
/// SSO session row. Common path for both `authorization_code` and
/// `refresh_token` grants.
async fn issue_token_pair(
    ctx: &AuthCtx,
    client: &super::types::OidcClient,
    user_id: &str,
    scopes: &[String],
    nonce: Option<&str>,
) -> Response {
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => {
            return token_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::SERVER_ERROR,
                None,
            );
        }
    };
    let user = match ctx.users.get_user_by_id(user_id).await {
        Ok(Some(u)) => Some(u),
        _ => None,
    };
    let email = user.as_ref().and_then(|u| u.email.clone());
    let email_verified = user.as_ref().map(|u| u.email_verified).unwrap_or(false);
    let display_name = user.as_ref().and_then(|u| u.display_name.clone());

    let sid = tok::mint_sid();

    let id_claims = tok::build_id_token_claims(
        &provider.issuer,
        user_id,
        &client.client_id,
        &sid,
        scopes,
        nonce,
        email.as_deref(),
        email_verified,
        display_name.as_deref(),
    );
    let access_claims =
        tok::build_access_token_claims(&provider.issuer, user_id, &client.client_id, &sid, scopes);

    let jwt = match ctx.jwt.as_ref() {
        Some(j) => j,
        None => {
            return token_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::SERVER_ERROR,
                Some("jwt not configured".into()),
            );
        }
    };
    let id_token = match jwt.issue(&id_claims) {
        Ok(t) => t,
        Err(e) => {
            return token_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::SERVER_ERROR,
                Some(format!("sign id_token: {e}")),
            );
        }
    };
    let access_token = match jwt.issue(&access_claims) {
        Ok(t) => t,
        Err(e) => {
            return token_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::SERVER_ERROR,
                Some(format!("sign access_token: {e}")),
            );
        }
    };

    // Refresh token issued when the client allows refresh_token grant
    // OR `offline_access` is in the requested scopes.
    let issue_refresh =
        client.allows_grant("refresh_token") || scopes.iter().any(|s| s == "offline_access");
    let refresh_token = if issue_refresh {
        let plaintext = tok::mint_refresh_token();
        let row = tok::build_refresh_row(user_id, &client.client_id, scopes, &plaintext);
        if let Err(e) = provider.refresh.create(&row).await {
            return token_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::SERVER_ERROR,
                Some(format!("persist refresh: {e}")),
            );
        }
        Some(plaintext)
    } else {
        None
    };

    // Record SSO session — back-channel logout fans out from this row.
    let oidc_session = OidcSession {
        sid: sid.clone(),
        user_id: user_id.to_string(),
        client_id: client.client_id.clone(),
        assay_session_id: None,
        issued_at: now_secs(),
        backchannel_logout_uri: client.backchannel_logout_uri.clone(),
    };
    if let Err(e) = provider.sessions.create(&oidc_session).await {
        tracing::warn!(?e, "failed to record SSO session — continuing");
    }

    let response = TokenResponse {
        access_token,
        token_type: "Bearer",
        expires_in: tok::ACCESS_TOKEN_LIFETIME_SECS as i64,
        id_token,
        refresh_token,
        scope: scopes.join(" "),
    };
    (StatusCode::OK, Json(response)).into_response()
}

// =====================================================================
//   /userinfo
// =====================================================================

/// `GET/POST /userinfo` — bearer access_token + scope-filtered claims.
pub async fn userinfo_get(State(ctx): State<AuthCtx>, headers: HeaderMap) -> Response {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(userinfo::parse_bearer);
    let Some(token) = bearer else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid_token"})),
        )
            .into_response();
    };
    let jwt = match ctx.jwt.as_ref() {
        Some(j) => j,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "server_error"})),
            )
                .into_response();
        }
    };
    let data = match jwt.verify::<AccessTokenClaims>(token) {
        Ok(d) => d,
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "invalid_token"})),
            )
                .into_response();
        }
    };
    let user = match ctx.users.get_user_by_id(&data.claims.sub).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "invalid_token"})),
            )
                .into_response();
        }
    };
    let claims = userinfo::build_userinfo(&user, &data.claims.scopes());
    (StatusCode::OK, Json(claims)).into_response()
}

// =====================================================================
//   /revoke
// =====================================================================

/// `POST /revoke` — RFC 7009. Always returns 200 (per spec).
pub async fn revoke_post(State(ctx): State<AuthCtx>, Form(req): Form<RevokeRequest>) -> Response {
    if let Some(provider) = ctx.oidc_provider.as_ref() {
        // Try as a refresh token (the one we can actually revoke).
        let hash = tok::hash_refresh_token(&req.token);
        let _ = provider.refresh.revoke(&hash).await;
    }
    StatusCode::OK.into_response()
}

// =====================================================================
//   /introspect
// =====================================================================

/// `POST /introspect` — RFC 7662. Validates client auth then returns
/// `{active, ...}` for known/active tokens or `{active: false}` for
/// anything else.
pub async fn introspect_post(
    State(ctx): State<AuthCtx>,
    headers: HeaderMap,
    Form(body): Form<IntrospectRequest>,
) -> Response {
    // Require client auth — synthesise a TokenRequest so the existing
    // helper does the parsing work.
    let synth = TokenRequest {
        grant_type: String::new(),
        ..Default::default()
    };
    if authenticate_client(&ctx, &headers, &synth).await.is_err() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(IntrospectResponse::inactive()),
        )
            .into_response();
    }

    let jwt = match ctx.jwt.as_ref() {
        Some(j) => j,
        None => return (StatusCode::OK, Json(IntrospectResponse::inactive())).into_response(),
    };

    // Try as an access_token JWT first.
    if let Ok(data) = jwt.verify::<AccessTokenClaims>(&body.token) {
        let resp = IntrospectResponse {
            active: true,
            client_id: Some(data.claims.client_id.clone()),
            username: Some(data.claims.sub.clone()),
            scope: Some(data.claims.scope.clone()),
            exp: Some(data.claims.exp),
            sub: Some(data.claims.sub.clone()),
            aud: Some(data.claims.aud.clone()),
            iat: Some(data.claims.iat),
            token_type: Some("Bearer".into()),
        };
        return (StatusCode::OK, Json(resp)).into_response();
    }

    // Not a JWT — try as an opaque refresh_token.
    if let Some(provider) = ctx.oidc_provider.as_ref() {
        let hash = tok::hash_refresh_token(&body.token);
        if let Ok(Some(row)) = provider.refresh.get(&hash).await
            && !row.revoked
            && row.expires_at > now_secs()
        {
            let resp = IntrospectResponse {
                active: true,
                client_id: Some(row.client_id.clone()),
                username: Some(row.user_id.clone()),
                scope: Some(row.scopes.join(" ")),
                exp: Some(row.expires_at as i64),
                sub: Some(row.user_id),
                aud: Some(row.client_id),
                iat: Some(row.issued_at as i64),
                token_type: Some("Bearer".into()),
            };
            return (StatusCode::OK, Json(resp)).into_response();
        }
    }

    (StatusCode::OK, Json(IntrospectResponse::inactive())).into_response()
}

// =====================================================================
//   /logout
// =====================================================================

/// Logout query params per OIDC RP-Initiated Logout 1.0.
#[derive(Deserialize)]
pub struct LogoutQuery {
    pub id_token_hint: Option<String>,
    pub post_logout_redirect_uri: Option<String>,
    pub state: Option<String>,
}

/// `GET /logout` — revoke the assay session, then redirect (or render).
pub async fn logout_get(
    State(ctx): State<AuthCtx>,
    headers: HeaderMap,
    Query(q): Query<LogoutQuery>,
) -> Response {
    if let Some(sid) = parse_cookie(&headers, crate::session::SESSION_COOKIE) {
        let _ = ctx.sessions.delete(&sid).await;
        // Fan out back-channel logout to every SSO session linked to
        // this assay session (best-effort).
        if let Some(provider) = ctx.oidc_provider.as_ref() {
            if let Ok(rows) = provider.sessions.list_by_assay_session(&sid).await {
                for row in rows {
                    if let Some(uri) = row.backchannel_logout_uri {
                        // Fire-and-forget: spawn a task per URI so the
                        // logout redirect doesn't block on slow clients.
                        tokio::spawn(async move {
                            let client = reqwest::Client::new();
                            let _ = client
                                .post(&uri)
                                .form(&[("logout_token", "stub")])
                                .timeout(std::time::Duration::from_secs(5))
                                .send()
                                .await;
                        });
                    }
                }
            }
            let _ = provider.sessions.delete_by_assay_session(&sid).await;
        }
    }
    let _ = q.id_token_hint;
    let _ = q.state;
    let target = q
        .post_logout_redirect_uri
        .unwrap_or_else(|| "/".to_string());
    let mut response = Redirect::to(&target).into_response();
    // Clear the session cookie.
    if let Ok(value) = format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        crate::session::SESSION_COOKIE
    )
    .parse()
    {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

// =====================================================================
//   /oidc/upstream/{slug}/start + /callback
// =====================================================================

/// Query for the federation start route.
#[derive(Deserialize)]
pub struct UpstreamStartQuery {
    pub return_to: Option<String>,
}

/// `GET /oidc/upstream/{slug}/start` — kick off federated login.
pub async fn upstream_start(
    State(ctx): State<AuthCtx>,
    Path(slug): Path<String>,
    Query(q): Query<UpstreamStartQuery>,
) -> Response {
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => return server_misconfigured("oidc_provider is not enabled"),
    };
    let registry = match ctx.oidc.as_ref() {
        Some(r) => r,
        None => return server_misconfigured("oidc client registry is not enabled"),
    };
    let started = match super::federation::start_upstream_login(
        registry,
        &provider.upstream_states,
        &slug,
        validate_return_to(q.return_to, &provider.public_url),
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            return error_html(StatusCode::BAD_REQUEST, &format!("upstream start: {e}"));
        }
    };
    let mut response = Redirect::to(&started.redirect_url).into_response();
    if let Ok(value) = build_binding_cookie(&started.binding_token, &provider.public_url).parse() {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

/// Query for the federation callback. `iss` is RFC 9207; lenient
/// (warn-on-missing, reject-on-mismatch).
#[derive(Deserialize)]
pub struct UpstreamCallbackQuery {
    pub code: String,
    pub state: String,
    pub iss: Option<String>,
}

/// `GET /oidc/upstream/{slug}/callback` — finish federated login.
pub async fn upstream_callback(
    State(ctx): State<AuthCtx>,
    headers: HeaderMap,
    Path(_slug): Path<String>,
    Query(q): Query<UpstreamCallbackQuery>,
) -> Response {
    let provider = match ctx.oidc_provider.as_ref() {
        Some(p) => p,
        None => return server_misconfigured("oidc_provider is not enabled"),
    };
    let registry = match ctx.oidc.as_ref() {
        Some(r) => r,
        None => return server_misconfigured("oidc client registry is not enabled"),
    };
    let binding_token = parse_cookie(&headers, UPSTREAM_BINDING_COOKIE);
    let info = match super::federation::complete_upstream_login(
        registry,
        &provider.upstream_states,
        &q.code,
        &q.state,
        binding_token.as_deref(),
        q.iss.as_deref(),
    )
    .await
    {
        Ok(i) => i,
        Err(e) => {
            let mut response =
                error_html(StatusCode::BAD_REQUEST, &format!("upstream complete: {e}"));
            append_clear_binding_cookie(&mut response, &provider.public_url);
            return response;
        }
    };

    // Look up or create the local user. Two regimes:
    //   auto_provision=true  → first sign-in for an upstream subject
    //                          creates an `auth.users` row from the
    //                          upstream claims (legacy / open-signup).
    //   auto_provision=false → invite-only. Look up by email; if no
    //                          row exists, return 403 — operators
    //                          pre-populate `auth.users` via the admin
    //                          API or the sysops `/auth/users` page.
    let user = match ctx
        .users
        .get_user_by_upstream(&info.provider_slug, &info.subject)
        .await
    {
        Ok(Some(u)) => u,
        Ok(None) => {
            let existing = if provider.auto_provision {
                None
            } else {
                // Invite-only lookup keyed on email. The upstream MUST
                // return one AND mark it verified — an attacker who
                // controls an upstream IdP that accepts unverified
                // email could otherwise claim an invited address and
                // link their upstream subject to the pre-created
                // local user.
                let email = match info.email.as_deref() {
                    Some(e) if !e.is_empty() => e,
                    _ => {
                        let mut response = error_html(
                            StatusCode::FORBIDDEN,
                            "upstream did not return an email claim; \
                             cannot match against the access list.",
                        );
                        append_clear_binding_cookie(&mut response, &provider.public_url);
                        return response;
                    }
                };
                if !info.email_verified {
                    let mut response = error_html(
                        StatusCode::FORBIDDEN,
                        "upstream returned an unverified email; \
                         cannot match against the access list. Verify \
                         the address with the upstream provider first.",
                    );
                    append_clear_binding_cookie(&mut response, &provider.public_url);
                    return response;
                }
                match ctx.users.get_user_by_email(email).await {
                    Ok(Some(u)) => Some(u),
                    Ok(None) => {
                        let mut response = error_html(
                            StatusCode::FORBIDDEN,
                            &format!(
                                "You signed in as {email}, but that account is \
                                 not yet authorised for this app. If you believe \
                                 this is a mistake, ask an administrator to invite \
                                 you."
                            ),
                        );
                        append_clear_binding_cookie(&mut response, &provider.public_url);
                        return response;
                    }
                    Err(e) => {
                        let mut response = server_error_html(&format!("user lookup by email: {e}"));
                        append_clear_binding_cookie(&mut response, &provider.public_url);
                        return response;
                    }
                }
            };
            let user = if let Some(u) = existing {
                // Invite-only: existing row, just link the upstream.
                u
            } else {
                let id = format!(
                    "usr_{}",
                    data_encoding::BASE64URL_NOPAD.encode(&random_bytes::<16>())
                );
                let user = crate::store::User {
                    id: id.clone(),
                    email: info.email.clone(),
                    email_verified: info.email_verified,
                    display_name: info.display_name.clone(),
                    created_at: now_secs(),
                };
                if let Err(e) = ctx.users.create_user(&user).await {
                    let mut response = server_error_html(&format!("create user: {e}"));
                    append_clear_binding_cookie(&mut response, &provider.public_url);
                    return response;
                }
                user
            };
            if let Err(e) = ctx
                .users
                .link_upstream(&user.id, &info.provider_slug, &info.subject)
                .await
            {
                let mut response = server_error_html(&format!("link upstream: {e}"));
                append_clear_binding_cookie(&mut response, &provider.public_url);
                return response;
            }
            user
        }
        Err(e) => {
            let mut response = server_error_html(&format!("upstream user lookup: {e}"));
            append_clear_binding_cookie(&mut response, &provider.public_url);
            return response;
        }
    };

    // Mint an assay session.
    let mgr = crate::session::SessionManager::with_default_duration(ctx.sessions.clone());
    let session = match mgr.create(&user.id).await {
        Ok(s) => s,
        Err(e) => {
            let mut response = server_error_html(&format!("create session: {e}"));
            append_clear_binding_cookie(&mut response, &provider.public_url);
            return response;
        }
    };
    let mut response = Redirect::to(info.return_to.as_deref().unwrap_or("/")).into_response();
    let cookie = crate::session::cookie_for(&session, &provider.public_url);
    if let Ok(value) = cookie.to_string().parse() {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    append_clear_binding_cookie(&mut response, &provider.public_url);
    response
}

/// Build the `Set-Cookie` value for the binding token. Omits `Secure`
/// only when the public URL is `http` to a localhost host (dev rigs).
/// Cookie `Path` for the binding cookie, derived from the OIDC public
/// URL so it matches whatever mount prefix the engine nests the spec
/// router under (typically `/auth`). Hardcoding `/oidc/upstream/` left
/// the cookie unsent on callbacks when the spec router was nested at
/// any prefix other than `/`.
fn binding_cookie_path(public_url: &url::Url) -> String {
    let base = public_url.path().trim_end_matches('/');
    format!("{base}/oidc/upstream/")
}

fn build_binding_cookie(raw: &str, public_url: &url::Url) -> String {
    let secure = !is_plain_http(public_url);
    let secure_attr = if secure { "; Secure" } else { "" };
    let path = binding_cookie_path(public_url);
    format!(
        "{UPSTREAM_BINDING_COOKIE}={raw}; Path={path}; HttpOnly; SameSite=Lax; \
         Max-Age=300{secure_attr}"
    )
}

/// Append a `Set-Cookie: …; Max-Age=0` header to clear the binding
/// cookie. Called on every callback response — the cookie's job ended
/// the moment the callback ran.
fn append_clear_binding_cookie(response: &mut Response, public_url: &url::Url) {
    let path = binding_cookie_path(public_url);
    let cleared =
        format!("{UPSTREAM_BINDING_COOKIE}=; Path={path}; Max-Age=0; HttpOnly; SameSite=Lax");
    if let Ok(value) = cleared.parse() {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
}

fn is_plain_http(url: &url::Url) -> bool {
    url.scheme() != "https"
}

/// Validate the `return_to` parameter passed to `/oidc/upstream/{slug}/start`.
/// Without this check, an attacker can craft a link to the legitimate
/// login surface that, after a successful upstream login, bounces the
/// victim to an attacker-controlled URL (open-redirect).
///
/// Acceptance rules:
///   - `None` / empty → `None`
///   - Path-only same-origin URLs (`/foo`, `/`) → accepted as-is
///   - Absolute URLs whose origin matches `public_url`'s origin → accepted
///   - Everything else (protocol-relative `//evil`, cross-origin
///     `https://evil`, schemes like `javascript:`) → `None`
fn validate_return_to(raw: Option<String>, public_url: &url::Url) -> Option<String> {
    let s = raw?;
    if s.is_empty() {
        return None;
    }
    // Reject protocol-relative URLs — `//evil/path` resolves to a
    // cross-origin redirect when browsers see it as `<current-scheme>://evil/path`.
    if s.starts_with("//") {
        return None;
    }
    // Accept same-origin path-only redirects.
    if s.starts_with('/') {
        return Some(s);
    }
    // Accept absolute URLs only if their origin matches ours.
    match url::Url::parse(&s) {
        Ok(u) if u.origin() == public_url.origin() => Some(s),
        _ => None,
    }
}

// =====================================================================
//   helpers
// =====================================================================

/// Encode the resume payload — base64-url of the JSON-serialised
/// AuthorizeRequest. Symmetric (no signing) — anchored on the
/// HttpOnly cookie + the CSRF token check.
fn encode_resume(req: &AuthorizeRequest) -> String {
    let json = serde_json::to_vec(req).unwrap_or_default();
    data_encoding::BASE64URL_NOPAD.encode(&json)
}

fn decode_resume(s: &str) -> Option<AuthorizeRequest> {
    let bytes = data_encoding::BASE64URL_NOPAD.decode(s.as_bytes()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Pull a single cookie value off the `Cookie` request header by name.
pub(crate) fn parse_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
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

/// `(StatusCode, Json<TokenErrorBody>)` builder for `/token` errors.
fn token_err(status: StatusCode, code: &str, desc: Option<String>) -> Response {
    (status, Json(err_body(code, desc))).into_response()
}

fn err_body(code: &str, desc: Option<String>) -> TokenErrorBody {
    TokenErrorBody {
        error: code.to_string(),
        error_description: desc,
    }
}

/// Render a plain-text error page with the given status. Used for the
/// non-redirect-safe authorize errors (bad client_id, bad redirect_uri).
fn error_html(status: StatusCode, message: &str) -> Response {
    let title = match status {
        StatusCode::FORBIDDEN => "Access denied",
        StatusCode::UNAUTHORIZED => "Sign-in required",
        StatusCode::BAD_REQUEST => "Bad request",
        StatusCode::INTERNAL_SERVER_ERROR => "Server error",
        _ => "Error",
    };
    let body = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{title}</title>
<style>
:root {{
  color-scheme: light dark;
  --bg: #0d1117; --card: #161b22; --text: #e6edf3; --muted: #8b949e;
  --accent: #e6662a; --border: #30363d;
}}
@media (prefers-color-scheme: light) {{
  :root {{
    --bg: #f6f8fa; --card: #ffffff; --text: #1f2328; --muted: #59636e;
    --accent: #cf5d27; --border: #d0d7de;
  }}
}}
html, body {{ height: 100%; }}
body {{
  margin: 0;
  background: var(--bg);
  color: var(--text);
  font: 14px -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
  display: flex; align-items: center; justify-content: center;
  padding: 1.5rem; box-sizing: border-box;
}}
.error-card {{
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: 12px;
  padding: 2.5rem 2.25rem;
  width: 100%; max-width: 420px;
  box-sizing: border-box;
  box-shadow: 0 8px 24px rgba(0,0,0,.24);
  text-align: center;
}}
h1 {{ margin: 0 0 1rem; font-size: 1.5rem; font-weight: 600; }}
p {{ margin: 0 0 1.5rem; color: var(--muted); line-height: 1.5; white-space: pre-wrap; word-break: break-word; }}
.actions {{ display: flex; gap: .5rem; justify-content: center; flex-wrap: wrap; }}
.button {{
  display: inline-block;
  padding: .65rem 1.15rem;
  border: 1px solid var(--border); border-radius: 8px;
  color: var(--text); text-decoration: none;
  font-weight: 500; font-size: .95rem;
  transition: border-color 120ms, background-color 120ms;
}}
.button:hover {{ border-color: var(--accent); background: rgba(230,102,42,.06); }}
</style>
</head>
<body>
<main class="error-card">
<h1>{title}</h1>
<p>{message}</p>
<div class="actions">
  <a class="button" href="/auth/login">Try a different account</a>
</div>
</main>
</body>
</html>"#,
        title = html_escape_simple(title),
        message = html_escape_simple(message),
    );
    (status, Html(body)).into_response()
}

/// Minimal HTML-escape for the strings interpolated into `error_html`.
/// Sufficient because we only interpolate into element text bodies, not
/// into attribute contexts, and the messages are operator-controlled.
fn html_escape_simple(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn server_error_html(message: &str) -> Response {
    error_html(StatusCode::INTERNAL_SERVER_ERROR, message)
}

fn server_misconfigured(reason: &str) -> Response {
    error_html(StatusCode::INTERNAL_SERVER_ERROR, reason)
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// URL-encode helper — narrow port of [`super::authorize::url_encode`];
/// duplicated here because that function is private to the authorize
/// module and we'd rather not expand its public surface for a tiny
/// helper.
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

fn random_bytes<const N: usize>() -> [u8; N] {
    use rand::RngCore;
    let mut buf = [0u8; N];
    rand::rng().fill_bytes(&mut buf);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cookie_handles_multi_pair_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "assay_session=sess_abc; assay_csrf=csrf_xyz; other=1"
                .parse()
                .unwrap(),
        );
        assert_eq!(
            parse_cookie(&headers, "assay_session").as_deref(),
            Some("sess_abc")
        );
        assert_eq!(
            parse_cookie(&headers, "assay_csrf").as_deref(),
            Some("csrf_xyz")
        );
        assert_eq!(parse_cookie(&headers, "missing"), None);
    }

    #[test]
    fn resume_round_trip() {
        let req = AuthorizeRequest {
            response_type: "code".into(),
            client_id: "c1".into(),
            redirect_uri: "https://app/cb".into(),
            scope: "openid email".into(),
            state: Some("s1".into()),
            nonce: None,
            code_challenge: Some("ch".into()),
            code_challenge_method: Some("S256".into()),
            prompt: None,
            max_age: None,
        };
        let encoded = encode_resume(&req);
        let decoded = decode_resume(&encoded).unwrap();
        assert_eq!(decoded, req);
    }

    #[test]
    fn verify_client_secret_handles_plaintext() {
        assert!(verify_client_secret("secret", "secret"));
        assert!(!verify_client_secret("wrong", "secret"));
        assert!(!verify_client_secret("secret", "differentlength"));
    }

    #[test]
    fn url_encode_handles_reserved_bytes() {
        assert_eq!(url_encode("a b/c"), "a%20b%2Fc");
        assert_eq!(url_encode("Plain-Text_1.0~"), "Plain-Text_1.0~");
    }

    fn return_to_issuer() -> url::Url {
        url::Url::parse("https://app.example.com/auth").unwrap()
    }

    #[test]
    fn validate_return_to_accepts_relative_path() {
        let u = return_to_issuer();
        assert_eq!(validate_return_to(Some("/".into()), &u), Some("/".into()));
        assert_eq!(
            validate_return_to(Some("/dashboard".into()), &u),
            Some("/dashboard".into())
        );
        assert_eq!(
            validate_return_to(Some("/a?b=c#d".into()), &u),
            Some("/a?b=c#d".into())
        );
    }

    #[test]
    fn validate_return_to_accepts_same_origin_absolute() {
        let u = return_to_issuer();
        let here = "https://app.example.com/some/path".to_string();
        assert_eq!(validate_return_to(Some(here.clone()), &u), Some(here));
    }

    #[test]
    fn validate_return_to_rejects_cross_origin() {
        let u = return_to_issuer();
        assert_eq!(
            validate_return_to(Some("https://evil.com".into()), &u),
            None
        );
        assert_eq!(
            validate_return_to(Some("https://evil.com/path".into()), &u),
            None
        );
        // Different subdomain — also cross-origin.
        assert_eq!(
            validate_return_to(Some("https://other.example.com/x".into()), &u),
            None
        );
    }

    #[test]
    fn validate_return_to_rejects_protocol_relative() {
        let u = return_to_issuer();
        assert_eq!(validate_return_to(Some("//evil.com/x".into()), &u), None);
        assert_eq!(validate_return_to(Some("//evil.com".into()), &u), None);
    }

    #[test]
    fn validate_return_to_rejects_javascript_and_data_schemes() {
        let u = return_to_issuer();
        assert_eq!(
            validate_return_to(Some("javascript:alert(1)".into()), &u),
            None
        );
        assert_eq!(
            validate_return_to(Some("data:text/html,<script>".into()), &u),
            None
        );
    }

    #[test]
    fn validate_return_to_passes_through_none_and_empty() {
        let u = return_to_issuer();
        assert_eq!(validate_return_to(None, &u), None);
        assert_eq!(validate_return_to(Some(String::new()), &u), None);
    }
}
