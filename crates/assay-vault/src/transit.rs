//! Transit — encrypt / decrypt without exposing key material to the
//! caller. Plan 17 §S2 / Vault-equivalent.
//!
//! Operators register a named key; clients call `encrypt` / `decrypt`
//! with the name and the server holds the underlying material. Rotating
//! a key bumps its version: subsequent `encrypt` calls use the new
//! version, but ciphertexts stamped with the old version remain
//! decryptable until an explicit re-wrap.
//!
//! ## Layering
//!
//! Mirrors KV: [`TransitStore`] is pure IO (raw wrapped key blobs +
//! version metadata), [`TransitService`] wraps `(store, KekHandle)`
//! and does the AEAD work.
//!
//! ## Wire format
//!
//! Ciphertexts are returned as ASCII strings: `vault:v{version}:{b64}`,
//! where the base64 payload is `nonce(12) || ciphertext(variable)` —
//! same shape Vault uses, easy to grep for in logs. Decrypt parses the
//! prefix to know which key version to fetch.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::crypto::aead::{decrypt, encrypt, random_dek, random_nonce, NONCE_LEN};
use crate::crypto::kek::WrappedDek;
use crate::error::{Result, VaultError};

/// Per-key metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TransitKey {
    pub name: String,
    pub algo: String,
    pub latest_ver: i64,
    pub created_at: f64,
}

/// One version of a transit key — DEK wrapped by the master KEK.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TransitVersion {
    pub name: String,
    pub version: i64,
    pub key_wrapped: Vec<u8>,
    pub kek_kid: String,
    pub created_at: f64,
}

/// Pure-IO trait. The PG / SQLite impls take and return raw wrapped-key
/// blobs; they don't unwrap.
#[async_trait]
pub trait TransitStore: Send + Sync + 'static {
    /// Insert a fresh key + its first version. `algo` is informational
    /// today — Phase 1 only ships AES-256-GCM-SIV.
    async fn create_key(
        &self,
        name: &str,
        algo: &str,
        version_wrapped: &[u8],
        kek_kid: &str,
    ) -> Result<()>;

    /// Read key metadata (returns None if the key doesn't exist).
    async fn get_key(&self, name: &str) -> Result<Option<TransitKey>>;

    /// Read one specific version (returns None if name/version doesn't exist).
    async fn get_version(&self, name: &str, version: i64) -> Result<Option<TransitVersion>>;

    /// Read the latest version. Convenience for the encrypt path.
    async fn get_latest_version(&self, name: &str) -> Result<Option<TransitVersion>>;

    /// Append a new version, bumping `latest_ver`. Returns the new
    /// version number.
    async fn rotate(&self, name: &str, version_wrapped: &[u8], kek_kid: &str) -> Result<i64>;

    /// List every transit key. Used by admin / dashboard.
    async fn list_keys(&self) -> Result<Vec<TransitKey>>;
}

/// High-level transit API. Cheap to clone. Defers to the live
/// [`crate::crypto::seal_state::SealState`] for KEK access — every
/// crypto op fails closed with [`VaultError::Sealed`] when the vault
/// is sealed.
#[derive(Clone)]
#[non_exhaustive]
pub struct TransitService<S: TransitStore> {
    store: S,
    seal_state: crate::crypto::seal_state::SealState,
}

impl<S: TransitStore> TransitService<S> {
    pub fn new(store: S, seal_state: crate::crypto::seal_state::SealState) -> Self {
        Self { store, seal_state }
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    /// Create a new transit key. Errors if the name already exists.
    /// `algo` is reserved for forward compatibility — Phase 1 always
    /// uses AES-256-GCM-SIV regardless of the value.
    pub async fn create_key(&self, name: &str, algo: Option<&str>) -> Result<()> {
        validate_name(name)?;
        let kek = self.seal_state.require_unsealed()?;
        let dek = random_dek();
        let wrapped = kek.wrap_dek(&dek)?;
        let algo = algo.unwrap_or("aes256-gcm-siv");
        self.store
            .create_key(name, algo, wrapped.as_bytes(), kek.kid())
            .await?;
        Ok(())
    }

    /// Encrypt `plaintext` with the current latest version of `name`.
    /// Returns the wire-format ciphertext (`vault:vN:b64...`).
    pub async fn encrypt(&self, name: &str, plaintext: &[u8]) -> Result<String> {
        validate_name(name)?;
        let kek = self.seal_state.require_unsealed()?;
        let v = self
            .store
            .get_latest_version(name)
            .await?
            .ok_or(VaultError::NotFound)?;
        let dek = unwrap_version(&kek, &v)?;
        let nonce = random_nonce();
        let aad = aad_for(name, v.version);
        let ct = encrypt(&dek, &nonce, &aad, plaintext)?;
        Ok(encode_envelope(v.version, &nonce, &ct))
    }

    /// Decrypt a wire-format ciphertext. Reads the version off the
    /// prefix, fetches that key version (which may be older than the
    /// current latest), and runs AEAD-decrypt.
    pub async fn decrypt(&self, name: &str, envelope: &str) -> Result<Vec<u8>> {
        validate_name(name)?;
        let kek = self.seal_state.require_unsealed()?;
        let parts = parse_envelope(envelope)?;
        let v = self
            .store
            .get_version(name, parts.version)
            .await?
            .ok_or(VaultError::NotFound)?;
        let dek = unwrap_version(&kek, &v)?;
        let aad = aad_for(name, parts.version);
        decrypt(&dek, &parts.nonce, &aad, &parts.ciphertext)
    }

    /// Append a new version to `name`. Returns the new version number.
    pub async fn rotate(&self, name: &str) -> Result<i64> {
        validate_name(name)?;
        let kek = self.seal_state.require_unsealed()?;
        let dek = random_dek();
        let wrapped = kek.wrap_dek(&dek)?;
        self.store
            .rotate(name, wrapped.as_bytes(), kek.kid())
            .await
    }

    pub async fn list_keys(&self) -> Result<Vec<TransitKey>> {
        self.store.list_keys().await
    }
}

fn unwrap_version(
    kek: &crate::crypto::kek::KekHandle,
    v: &TransitVersion,
) -> Result<[u8; 32]> {
    if v.kek_kid != kek.kid() {
        return Err(VaultError::Crypto(format!(
            "transit version {name}/v{ver} encrypted with KEK {kid} but service active KEK is {active}",
            name = v.name,
            ver = v.version,
            kid = v.kek_kid,
            active = kek.kid()
        )));
    }
    kek.unwrap_dek(&WrappedDek::from_bytes(v.key_wrapped.clone()))
}

/// Bind name + version into the AEAD AAD. A cipher decrypted under the
/// wrong key version (e.g. v2's DEK against v3's ciphertext) would
/// otherwise decrypt successfully if the DEKs collide; the AAD makes
/// that impossible.
fn aad_for(name: &str, version: i64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(name.len() + 32);
    buf.extend_from_slice(b"vault.transit:");
    buf.extend_from_slice(name.as_bytes());
    buf.extend_from_slice(b":v");
    buf.extend_from_slice(version.to_string().as_bytes());
    buf
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(VaultError::Invalid("transit key name is empty".into()));
    }
    if name.len() > 256 {
        return Err(VaultError::Invalid("transit key name > 256 bytes".into()));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        return Err(VaultError::Invalid(
            "transit key name: only [A-Za-z0-9._/-] allowed".into(),
        ));
    }
    Ok(())
}

struct EnvelopeParts {
    version: i64,
    nonce: [u8; NONCE_LEN],
    ciphertext: Vec<u8>,
}

fn encode_envelope(version: i64, nonce: &[u8; NONCE_LEN], ct: &[u8]) -> String {
    let mut buf = Vec::with_capacity(NONCE_LEN + ct.len());
    buf.extend_from_slice(nonce);
    buf.extend_from_slice(ct);
    let b64 = data_encoding::BASE64.encode(&buf);
    format!("vault:v{version}:{b64}")
}

fn parse_envelope(s: &str) -> Result<EnvelopeParts> {
    let rest = s
        .strip_prefix("vault:v")
        .ok_or_else(|| VaultError::Invalid("missing vault:v prefix".into()))?;
    let (ver_str, b64) = rest
        .split_once(':')
        .ok_or_else(|| VaultError::Invalid("missing version separator".into()))?;
    let version: i64 = ver_str
        .parse()
        .map_err(|_| VaultError::Invalid(format!("bad version '{ver_str}'")))?;
    let raw = data_encoding::BASE64
        .decode(b64.as_bytes())
        .map_err(|_| VaultError::Invalid("bad base64 in envelope".into()))?;
    if raw.len() < NONCE_LEN {
        return Err(VaultError::Invalid("envelope shorter than nonce".into()));
    }
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&raw[..NONCE_LEN]);
    let ciphertext = raw[NONCE_LEN..].to_vec();
    Ok(EnvelopeParts {
        version,
        nonce,
        ciphertext,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_round_trip() {
        let v = 7;
        let nonce = [9u8; NONCE_LEN];
        let ct = vec![1, 2, 3, 4];
        let s = encode_envelope(v, &nonce, &ct);
        assert!(s.starts_with("vault:v7:"));
        let p = parse_envelope(&s).unwrap();
        assert_eq!(p.version, 7);
        assert_eq!(p.nonce, nonce);
        assert_eq!(p.ciphertext, ct);
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_envelope("hello").is_err());
        assert!(parse_envelope("vault:v:abc").is_err());
        assert!(parse_envelope("vault:vX:abc").is_err());
        assert!(parse_envelope("vault:v1:!!!").is_err());
    }

    #[test]
    fn validate_name_accepts_paths_and_punctuation() {
        assert!(validate_name("logs").is_ok());
        assert!(validate_name("svc/api/access-token").is_ok());
        assert!(validate_name("v1.2_internal").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("a b").is_err());
        assert!(validate_name("nope!").is_err());
    }

    #[test]
    fn aad_distinguishes_versions() {
        assert_ne!(aad_for("k", 1), aad_for("k", 2));
        assert_ne!(aad_for("a", 1), aad_for("b", 1));
    }
}
