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

    // Plan §S6 — two-step auth detection. If the user has registered
    // any passkey credentials in assay-auth, refuse password-only
    // login and return BW's TwoFactorProviders shape so the client
    // prompts for the second factor.
    //
    // BW's TwoFactorProviders is keyed by integer enum:
    //   1 = Email, 2 = Authenticator (TOTP), 3 = Duo, 4 = YubiKey,
    //   5 = U2F (deprecated), 6 = Remember, 7 = WebAuthn.
    //
    // assay-auth's webauthn-rs surface is the FIDO2 path — type 7.
    // The actual second-factor challenge / response runs through the
    // existing /api/v1/engine/auth/passkey/login ceremony; the BW
    // client follows the same shape for type=7. Phase-7 v0.3.0 ships
    // the *detection*; full BW WebAuthn round-trip via the BW client
    // protocol lands in v0.3.x once the BW WebAuthn JSON wire-format
    // adapter is in place.
    if user_has_passkeys(&auth, &user.id).await {
        // BW expects this exact 400 + body shape so the client knows
        // to prompt for 2FA rather than re-prompting the password.
        let two_factor = serde_json::json!({
            "error": "invalid_grant",
            "error_description": "Two factor required.",
            "TwoFactorProviders": ["7"],
            "TwoFactorProviders2": {
                "7": {}
            }
        });
        return (StatusCode::BAD_REQUEST, axum::Json(two_factor)).into_response();
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
    // iss + aud must match the JwtConfig's own values — verify path
    // (assay-auth's JwtConfig::verify) reuses them as the validation
    // set, so a token minted with mismatched values fails its own
    // verifier.
    let claims = BwClaims {
        sub: &user.id,
        iss: jwt.issuer(),
        aud: jwt.audience(),
        iat: now,
        exp: now + 3600,
        scope: body
            .scope
            .clone()
            .unwrap_or_else(|| "api offline_access".into()),
    };
    let token = match jwt.issue(&claims) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(target: "assay-vault.bw", ?e, "bw connect_token: jwt issue failed");
            return error("server_error", "jwt issue failed");
        }
    };
    // Plan §"Open questions" #1: Argon2id default for new accounts.
    // Parameters mirror `assay_auth::password::PasswordHasher::default()`
    // so the BW client's locally-derived master-key hash and the
    // server's Argon2id round share KDF posture.
    let resp = TokenResponse {
        access_token: token,
        expires_in: 3600,
        token_type: "Bearer".into(),
        refresh_token: None,
        private_key: None,
        kdf: 1,            // Argon2id
        kdf_iterations: 3, // t_cost
        kdf_memory: 64,    // MiB
        kdf_parallelism: 4,
    };
    axum::Json(resp).into_response()
}

/// Whether the user has any passkey credentials registered with
/// assay-auth. Used to decide if the BW shim needs to demand 2FA.
async fn user_has_passkeys(auth: &AuthCtx, user_id: &str) -> bool {
    auth.users
        .list_passkeys(user_id)
        .await
        .map(|v| !v.is_empty())
        .unwrap_or(false)
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
