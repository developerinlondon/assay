//! Workflow API-key helpers.
//!
//! Up through plan-15 slice 1, this module also held the workflow's
//! own auth middleware (`AuthMode` + JWT/api-key dispatch + bootstrap
//! window). Slice 2 lifted authentication to the engine layer
//! (`assay_auth::gate`) — workflow no longer carries its own gate.
//! What's left here are the api-key bytes-and-strings helpers used by
//! [`crate::api::api_keys`] until the api-key surface itself goes away
//! in slice 3.

use sha2::{Digest, Sha256};

/// SHA-256 of the api-key bytes, lowercased hex. Used as the storage
/// key and lookup form so the plaintext key never reaches the DB.
pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    data_encoding::HEXLOWER.encode(&hasher.finalize())
}

/// Mint a fresh api-key (`assay_<64-hex>`) — 32 bytes of randomness
/// from the OS RNG, hex-encoded for cut-and-paste safety.
pub fn generate_api_key() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::rng().random();
    format!("assay_{}", data_encoding::HEXLOWER.encode(&bytes))
}

/// Display-only prefix (`assay_<8-hex>...`) for the dashboard's "this
/// is the key you'll see in the UI after creation" message. The full
/// plaintext is shown ONCE on creation; subsequent reads only ever
/// surface this prefix + the storage hash.
pub fn key_prefix(key: &str) -> String {
    let stripped = key.strip_prefix("assay_").unwrap_or(key);
    format!("assay_{}...", &stripped[..8.min(stripped.len())])
}
