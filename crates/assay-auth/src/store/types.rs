//! Plain-old-data records persisted by the auth stores.
//!
//! Field shapes are deliberately database-agnostic: timestamps are
//! `f64` seconds since UNIX epoch (matches the engine's existing
//! convention), opaque ids/credential bytes are owned `String`/`Vec<u8>`.

use serde::{Deserialize, Serialize};

/// Authoritative user record. The `id` is opaque — typically a
/// `usr_<base64url>` string minted at signup time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub display_name: Option<String>,
    pub created_at: f64,
}

/// One stored WebAuthn credential. `transports` is a comma-separated
/// list (CSV) per the `auth.passkeys.transports` column shape — the
/// PG/SQLite stores serialise this on write.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PasskeyCred {
    pub credential_id: Vec<u8>,
    pub public_key: Vec<u8>,
    pub sign_count: u32,
    pub transports: Vec<String>,
    pub created_at: f64,
    /// Full serialised [`webauthn_rs::prelude::Passkey`] JSON blob. This
    /// is the *authoritative* re-verification material: the server feeds
    /// it (carrying the persisted sign-count) back into the library on
    /// the authentication ceremony so counter / clone-detection works.
    /// `None` only for legacy rows written before the column existed —
    /// those rows can no longer drive a discoverable login and must be
    /// re-registered.
    #[serde(default)]
    pub passkey_json: Option<String>,
}

/// Opaque server-side session. `id` is the cookie value the client
/// presents on every request; `csrf_token` is sent in a parallel
/// non-HttpOnly cookie and must match a header/form field on
/// state-changing requests (double-submit pattern).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub csrf_token: String,
    pub created_at: f64,
    pub expires_at: f64,
    pub ip_hash: Option<String>,
    pub user_agent_hash: Option<String>,
}
