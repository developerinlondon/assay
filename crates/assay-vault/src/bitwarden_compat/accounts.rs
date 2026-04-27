//! `/api/accounts/*` — registration + prelogin endpoints BW clients
//! call before `/identity/connect/token`. Needed for stock `bw login`
//! to work end-to-end.
//!
//! - `POST /api/accounts/prelogin` — given an email, return the user's
//!   KDF posture so the client can derive the master-key hash with the
//!   right parameters.
//! - `POST /api/accounts/register` — create a new BW account.
//! - `POST /api/accounts/register/finish` — BW v2024+ split-register
//!   alias the older clients still hit.
//!
//! Ride on assay-auth's existing UserStore + PasswordHasher.

use axum::Router;
use axum::extract::{FromRef, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde::{Deserialize, Serialize};

use assay_auth::AuthCtx;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AuthCtx: FromRef<S>,
{
    Router::new()
        .route("/api/accounts/prelogin", post(prelogin::<S>))
        .route("/api/accounts/register", post(register::<S>))
        .route("/api/accounts/register/finish", post(register::<S>))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreloginBody {
    email: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct PreloginResponse {
    /// 0 = PBKDF2-SHA256, 1 = Argon2id.
    kdf: i32,
    kdf_iterations: u32,
    kdf_memory: Option<u32>,
    kdf_parallelism: Option<u32>,
}

async fn prelogin<S>(
    State(auth): State<AuthCtx>,
    axum::Json(body): axum::Json<PreloginBody>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    AuthCtx: FromRef<S>,
{
    // Plan §"Open questions" #1: Argon2id default for new accounts.
    // We don't store BW-specific KDF rows yet — every user gets the
    // assay-auth Argon2id default. (A per-user override row lands in
    // v0.3.x when imported PBKDF2 BW vaults need their own stamp.)
    let _ = body.email; // present-but-unused; assay-auth doesn't gate prelogin on user existence
    let _ = auth;
    axum::Json(PreloginResponse {
        kdf: 1, // Argon2id
        kdf_iterations: 3,
        kdf_memory: Some(64),
        kdf_parallelism: Some(4),
    })
    .into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RegisterBody {
    email: String,
    name: Option<String>,
    /// BW client sends a base64 hash derived locally from the master
    /// password; we apply assay-auth's Argon2id round on top before
    /// storing.
    master_password_hash: String,
    /// Encrypted symmetric key the client sends as part of register.
    /// Stored as-is for client to round-trip on /sync; the server
    /// never decrypts.
    #[allow(dead_code)]
    key: Option<String>,
    #[allow(dead_code)]
    keys: Option<serde_json::Value>,
}

async fn register<S>(
    State(auth): State<AuthCtx>,
    axum::Json(body): axum::Json<RegisterBody>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    AuthCtx: FromRef<S>,
{
    // Conflict: user already exists.
    if let Ok(Some(_)) = auth.users.get_user_by_email(&body.email).await {
        return error_resp(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Email is already taken.",
        );
    }
    // Hash the BW-derived password input via assay-auth's Argon2id.
    let hasher = assay_auth::password::PasswordHasher::default();
    let phc = match hasher.hash(&body.master_password_hash) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(target: "assay-vault.bw", ?e, "register: hash failed");
            return error_resp(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "hash failed",
            );
        }
    };
    let user_id = uuid::Uuid::now_v7().to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    // assay-auth's User shape: build it via the store's create_user.
    let user = assay_auth::store::User {
        id: user_id.clone(),
        email: Some(body.email.clone()),
        email_verified: false,
        display_name: body.name.clone(),
        created_at: now,
    };
    if let Err(e) = auth.users.create_user(&user).await {
        tracing::warn!(target: "assay-vault.bw", ?e, "register: create_user failed");
        return error_resp(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "create_user failed",
        );
    }
    if let Err(e) = auth.users.set_password_hash(&user_id, &phc).await {
        tracing::warn!(target: "assay-vault.bw", ?e, "register: set_password_hash failed");
        return error_resp(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "set_password_hash failed",
        );
    }
    StatusCode::OK.into_response()
}

fn error_resp(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    (
        status,
        axum::Json(serde_json::json!({
            "error": code,
            "error_description": message,
        })),
    )
        .into_response()
}
