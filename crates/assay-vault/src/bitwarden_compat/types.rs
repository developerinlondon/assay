//! Bitwarden JSON wire types.
//!
//! Field names use Pascal-case via `#[serde(rename_all = "PascalCase")]`
//! because BW's API serialises that way. The shim accepts the input,
//! stores the relevant fields against `vault.items` / `vault.folders`,
//! and returns them in the same shape.

use serde::{Deserialize, Serialize};

/// Bitwarden cipher type code.
#[allow(dead_code)]
pub mod cipher_type {
    pub const LOGIN: i32 = 1;
    pub const SECURE_NOTE: i32 = 2;
    pub const CARD: i32 = 3;
    pub const IDENTITY: i32 = 4;
    pub const SSH_KEY: i32 = 5;
}

/// Profile of the currently-authenticated user — what
/// `GET /api/accounts/profile` returns.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
#[non_exhaustive]
pub struct Profile {
    pub id: String,
    pub email: String,
    pub email_verified: bool,
    pub name: Option<String>,
    pub premium: bool,
    pub culture: String,
    /// JSON-encoded crypto keys; clients send their own. We round-trip
    /// what they sent (assay-auth doesn't currently store BW crypto
    /// material, so this stays as the BW client manages it client-side).
    pub key: Option<String>,
    pub private_key: Option<String>,
    pub security_stamp: String,
    #[serde(rename = "Object")]
    pub object: &'static str,
}

/// One item / cipher — BW's actual wire shape. Each type-specific
/// field (Login, SecureNote, Card, Identity) holds client-pre-encrypted
/// `encString`-format strings that ride through the server as
/// opaque JSON. The server never decrypts; it just round-trips the
/// JSON via items.ciphertext.
///
/// Passkey-as-cipher (plan §S6) lives in `Login.Fido2Credentials` — an
/// array of FIDO2 credential objects each containing client-encrypted
/// `credentialId`, `keyType`, `keyAlgorithm`, etc. The cipher's
/// `item_type` stays 1 (Login); the Fido2Credentials array distinguishes.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
#[non_exhaustive]
pub struct Cipher {
    pub id: String,
    pub user_id: Option<String>,
    pub organization_id: Option<String>,
    pub folder_id: Option<String>,
    /// 1 = Login, 2 = SecureNote, 3 = Card, 4 = Identity, 5 = SshKey.
    #[serde(rename = "Type")]
    pub item_type: i32,
    pub name: String,
    pub notes: Option<String>,
    pub login: Option<serde_json::Value>,
    pub secure_note: Option<serde_json::Value>,
    pub card: Option<serde_json::Value>,
    pub identity: Option<serde_json::Value>,
    #[serde(rename = "SshKey")]
    pub ssh_key: Option<serde_json::Value>,
    pub favorite: bool,
    pub revision_date: String,
    #[serde(rename = "Object")]
    pub object: &'static str,
}

/// What the client POSTs / PUTs to /api/ciphers — BW's wire shape.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
#[non_exhaustive]
pub struct CipherInput {
    pub folder_id: Option<String>,
    #[serde(rename = "Type")]
    pub item_type: i32,
    pub name: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub login: Option<serde_json::Value>,
    #[serde(default)]
    pub secure_note: Option<serde_json::Value>,
    #[serde(default)]
    pub card: Option<serde_json::Value>,
    #[serde(default)]
    pub identity: Option<serde_json::Value>,
    #[serde(default, rename = "SshKey")]
    pub ssh_key: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
#[non_exhaustive]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub revision_date: String,
    #[serde(rename = "Object")]
    pub object: &'static str,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
#[non_exhaustive]
pub struct FolderInput {
    pub name: String,
}

/// What `GET /api/sync` returns — the full vault dump for a user.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
#[non_exhaustive]
pub struct SyncResponse {
    pub profile: Profile,
    pub folders: Vec<Folder>,
    pub ciphers: Vec<Cipher>,
    pub collections: Vec<serde_json::Value>,
    pub policies: Vec<serde_json::Value>,
    pub sends: Vec<serde_json::Value>,
    pub domains: serde_json::Value,
    #[serde(rename = "Object")]
    pub object: &'static str,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
#[non_exhaustive]
pub struct ConnectTokenForm {
    pub grant_type: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub scope: Option<String>,
    pub client_id: Option<String>,
    /// Client-derived; BW clients send "0" for SHA-256 device ID
    /// hash, the actual KDF iterations are negotiated via prelogin.
    #[allow(dead_code)]
    pub device_identifier: Option<String>,
    #[allow(dead_code)]
    pub device_name: Option<String>,
    #[allow(dead_code)]
    pub device_type: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[non_exhaustive]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: u64,
    pub token_type: String,
    pub refresh_token: Option<String>,
    /// PrivateKey blob the client expects; we mirror what they had on
    /// register if available.
    #[serde(rename = "PrivateKey")]
    pub private_key: Option<String>,
    /// Master-key Kdf info. BW clients use this to derive keys from
    /// the master password locally before sending the derived hash
    /// to the server. type=0 is PBKDF2-SHA256, type=1 is Argon2id.
    ///
    /// Plan §"Open questions" #1 locks Argon2id as the default for
    /// new accounts (matches assay-auth's own password-hash storage).
    /// Imported BW vaults that were originally PBKDF2-SHA256 ride
    /// through unchanged — clients negotiate via /api/accounts/prelogin
    /// to read the per-user Kdf row when one exists.
    #[serde(rename = "Kdf")]
    pub kdf: i32,
    /// Argon2id `t_cost` (number of passes). Matches assay-auth's
    /// `DEFAULT_TIME_COST = 3`.
    #[serde(rename = "KdfIterations")]
    pub kdf_iterations: u32,
    /// Argon2id memory cost in MiB. BW's UI calls this "Memory";
    /// 64 MiB matches assay-auth's `DEFAULT_MEMORY_KIB / 1024`.
    #[serde(rename = "KdfMemory")]
    pub kdf_memory: u32,
    /// Argon2id parallelism. 4 threads matches assay-auth's
    /// `DEFAULT_PARALLELISM`.
    #[serde(rename = "KdfParallelism")]
    pub kdf_parallelism: u32,
}
