//! `/identity/connect/token` — OAuth2 password grant.
//!
//! Bitwarden clients POST their email + a PBKDF2-derived hash of the
//! master password as the password parameter. The server validates
//! against the user's stored Argon2id (or PBKDF2-SHA256) password
//! hash via assay-auth's password module; on success it issues a
//! JWT minted by assay-auth's JwtConfig.
//!
//! This is the BW-protocol shape mapped onto assay-auth's existing
//! identity surface. Bitwarden-derived hashes round-trip transparently
//! through the assay-auth password store — assay-auth doesn't know
//! the password came from a BW client, only that the input matches.

use axum::Router;
use axum::extract::{FromRef, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde::Serialize;

use assay_auth::AuthCtx;

use super::types::{ConnectTokenForm, TokenResponse};

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AuthCtx: FromRef<S>,
{
    Router::new().route("/identity/connect/token", post(connect_token::<S>))
}

/// JWT claim set we issue for BW clients. The `sub` carries the
/// assay-auth user id; `aud` lets the verifier check intent.
#[derive(Serialize)]
struct BwClaims<'a> {
    sub: &'a str,
    iss: String,
    aud: Vec<String>,
    iat: u64,
    exp: u64,
    scope: String,
}

async fn connect_token<S>(
    State(auth): State<AuthCtx>,
    axum::extract::Form(body): axum::extract::Form<ConnectTokenForm>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    AuthCtx: FromRef<S>,
{
    if body.grant_type != "password" {
        return error("unsupported_grant_type", "only password grant supported");
    }
    let email = match body.username {
        Some(e) => e,
        None => return error("invalid_request", "username required"),
    };
    let password = match body.password {
        Some(p) => p,
        None => return error("invalid_request", "password required"),
    };

    // Look up user.
    let user = match auth.users.get_user_by_email(&email).await {
        Ok(Some(u)) => u,
        Ok(None) => return error("invalid_grant", "invalid_username_or_password"),
        Err(e) => {
            tracing::warn!(target: "assay-vault.bw", ?e, %email, "bw connect_token: lookup failed");
            return error("server_error", "lookup failed");
        }
    };
    let stored_hash = match auth.users.get_password_hash(&user.id).await {
        Ok(Some(h)) => h,
        _ => return error("invalid_grant", "invalid_username_or_password"),
    };
    // Verify via assay-auth's password module — works against
    // Argon2id PHC strings stored in auth.users.password_hash. BW
    // clients send their KDF-derived bytes as the password input;
    // assay-auth applies its own Argon2id round.
    let hasher = assay_auth::password::PasswordHasher::default();
    let ok = hasher.verify(&password, &stored_hash).unwrap_or(false);
    if !ok {
        return error("invalid_grant", "invalid_username_or_password");
    }

    // Mint a JWT via the existing JwtConfig.
    let jwt = match auth.jwt.as_ref() {
        Some(j) => j,
        None => return error("server_error", "JWT issuer not configured"),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let claims = BwClaims {
        sub: &user.id,
        iss: "assay-vault/bw-compat".into(),
        aud: vec!["assay-vault".into()],
        iat: now,
        exp: now + 3600,
        scope: body.scope.clone().unwrap_or_else(|| "api offline_access".into()),
    };
    let token = match jwt.issue(&claims) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(target: "assay-vault.bw", ?e, "bw connect_token: jwt issue failed");
            return error("server_error", "jwt issue failed");
        }
    };
    let resp = TokenResponse {
        access_token: token,
        expires_in: 3600,
        token_type: "Bearer".into(),
        refresh_token: None,
        private_key: None,
        kdf: 0,
        kdf_iterations: 600_000,
    };
    axum::Json(resp).into_response()
}

fn error(code: &'static str, message: &'static str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(serde_json::json!({
            "error": code,
            "error_description": message,
        })),
    )
        .into_response()
}
