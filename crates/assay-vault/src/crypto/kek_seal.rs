//! KEK sealing under operator-supplied unseal material.
//!
//! Closes #113 ("vault master key stored in plaintext next to the data
//! it encrypts"). The master KEK is the root of the vault's key
//! hierarchy: every per-record DEK, transit-key version, and collection
//! envelope is wrapped under it. Storing it in plaintext meant a single
//! DB read decrypted every secret. This module seals it at rest under a
//! key the operator supplies out-of-band (env var, file, or — for the
//! Shamir / KMS phases — a different code path entirely).
//!
//! ## At-rest format (`sealing_method = 'sealed-v1'`)
//!
//! The `vault.kek_metadata.sealed_blob` for a sealed row is:
//!
//! ```text
//! version (1) | kdf (1) | salt_len (1) | salt (salt_len) | nonce (12) | ciphertext (48)
//! ```
//!
//! - `version` — format tag. `1` today; future KMS/HSM phases bump it so
//!   an old binary refuses a blob it can't interpret instead of
//!   mis-parsing.
//! - `kdf` — how the operator material maps to the 32-byte wrapping key:
//!   - `0x00` [`KDF_RAW`]: the material IS a 32-byte high-entropy key
//!     (base64-decoded). No salt; `salt_len = 0`.
//!   - `0x01` [`KDF_ARGON2ID`]: the material is a passphrase run through
//!     Argon2id (m=64MiB, t=3, p=4 — the same params assay-auth uses for
//!     password hashing) with the embedded random salt.
//! - `nonce` / `ciphertext` — AES-256-GCM-SIV (the vault's only AEAD).
//!   The ciphertext is the 32-byte KEK plus the 16-byte auth tag. AAD
//!   binds the format version + the KEK `kid` so a blob can't be lifted
//!   onto a different row.
//!
//! ## Why this shape
//!
//! - Reuses the existing AEAD ([`crate::crypto::aead`]) — "are we doing
//!   AEAD correctly?" stays a one-file question.
//! - The version tag makes the Shamir / KMS / HSM phases additive: they
//!   ship new `sealing_method` strings + new blob versions without
//!   touching this path.
//! - Wrong unseal material fails at the AEAD tag check — there is no
//!   plaintext fallback, no oracle, no silent garbage KEK.

use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;
use zeroize::Zeroizing;

use crate::crypto::aead::{KEY_LEN, NONCE_LEN, decrypt, encrypt, random_nonce};
use crate::error::{Result, VaultError};

/// `sealing_method` column value for a KEK sealed by this module.
pub const METHOD_SEALED_V1: &str = "sealed-v1";

/// Blob format version. Bumped by future KMS/HSM phases.
const SEAL_BLOB_VERSION: u8 = 1;

/// KDF tag: the unseal material is raw 32-byte key material (base64).
const KDF_RAW: u8 = 0x00;
/// KDF tag: the unseal material is a passphrase run through Argon2id.
const KDF_ARGON2ID: u8 = 0x01;

/// Argon2id memory cost in KiB — 64 MiB. Matches assay-auth's password
/// hasher (`crates/assay-auth/src/password.rs`), above the OWASP 2024
/// minimum. Kept in sync deliberately: one cost envelope for the whole
/// product.
const ARGON2_MEMORY_KIB: u32 = 65_536;
/// Argon2id time cost (passes).
const ARGON2_TIME_COST: u32 = 3;
/// Argon2id parallelism.
const ARGON2_PARALLELISM: u32 = 4;
/// Salt length for the Argon2id path, bytes.
const ARGON2_SALT_LEN: usize = 16;

/// Domain-separation prefix for the seal AEAD's AAD. The full AAD is
/// `SEAL_AAD_PREFIX || kid` so a sealed blob is bound to its KEK kid.
const SEAL_AAD_PREFIX: &[u8] = b"assay-vault/kek-seal/v1:";

/// Operator-supplied material that unseals the KEK. Resolved from config
/// before any DB work so a missing/unreadable source fails boot fast and
/// loud, never silently.
///
/// Variants intentionally hold the material by value so the caller can
/// drop it as soon as seal/unseal returns. The bytes are zeroized on
/// drop via [`Zeroizing`].
pub enum UnsealMaterial {
    /// High-entropy 32-byte key (preferred). Used verbatim as the
    /// AES-256-GCM-SIV wrapping key — no KDF.
    RawKey(Zeroizing<[u8; KEY_LEN]>),
    /// A passphrase. Stretched to a 32-byte wrapping key via Argon2id.
    Passphrase(Zeroizing<String>),
}

// Manual, redacting Debug — the material is a secret and must never
// land in logs or panic messages. Reveals only the variant kind.
impl std::fmt::Debug for UnsealMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RawKey(_) => f.write_str("UnsealMaterial::RawKey(<redacted>)"),
            Self::Passphrase(_) => f.write_str("UnsealMaterial::Passphrase(<redacted>)"),
        }
    }
}

impl UnsealMaterial {
    /// Parse a base64 string as a raw 32-byte key. The preferred source
    /// shape: `ASSAY_VAULT_UNSEAL_KEY=$(openssl rand -base64 32)`.
    ///
    /// Accepts standard and URL-safe base64, with or without padding —
    /// operators paste from many tools. Rejects anything that doesn't
    /// decode to exactly 32 bytes with an actionable error (so a
    /// truncated paste is caught at boot, not at first unwrap).
    pub fn raw_key_from_base64(s: &str) -> Result<Self> {
        let trimmed = s.trim();
        let raw = decode_base64_any(trimmed).ok_or_else(|| {
            VaultError::Invalid(
                "ASSAY_VAULT_UNSEAL_KEY: value is not valid base64. Provide a base64-encoded \
                 32-byte key, e.g. `openssl rand -base64 32`, or set the source kind to \
                 `passphrase:` for a human passphrase."
                    .into(),
            )
        })?;
        if raw.len() != KEY_LEN {
            return Err(VaultError::Invalid(format!(
                "ASSAY_VAULT_UNSEAL_KEY: decoded to {} bytes; a raw unseal key must be exactly \
                 {KEY_LEN} bytes (base64 of 32 random bytes). For a human-memorable secret use a \
                 `passphrase:` source instead.",
                raw.len()
            )));
        }
        let mut key = [0u8; KEY_LEN];
        key.copy_from_slice(&raw);
        Ok(Self::RawKey(Zeroizing::new(key)))
    }

    /// Treat the string as a passphrase (stretched via Argon2id). Rejects
    /// empty / whitespace-only passphrases.
    pub fn passphrase(s: impl Into<String>) -> Result<Self> {
        let pw = s.into();
        if pw.trim().is_empty() {
            return Err(VaultError::Invalid(
                "vault unseal passphrase is empty; refusing to seal the KEK under an empty \
                 passphrase"
                    .into(),
            ));
        }
        Ok(Self::Passphrase(Zeroizing::new(pw)))
    }

    /// Derive the 32-byte wrapping key for the seal AEAD, given the KDF
    /// tag + salt that were (or will be) stored in the blob.
    fn wrapping_key(
        &self,
        kdf: u8,
        salt: &[u8],
    ) -> Result<Zeroizing<[u8; KEY_LEN]>> {
        match (self, kdf) {
            (Self::RawKey(k), KDF_RAW) => Ok(Zeroizing::new(**k)),
            (Self::Passphrase(pw), KDF_ARGON2ID) => {
                let params = Params::new(ARGON2_MEMORY_KIB, ARGON2_TIME_COST, ARGON2_PARALLELISM, Some(KEY_LEN))
                    .map_err(|e| VaultError::Crypto(format!("argon2 params: {e}")))?;
                let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
                let mut out = Zeroizing::new([0u8; KEY_LEN]);
                argon
                    .hash_password_into(pw.as_bytes(), salt, out.as_mut_slice())
                    .map_err(|e| VaultError::Crypto(format!("argon2 derive: {e}")))?;
                Ok(out)
            }
            // A blob's KDF tag must match the material kind. This only
            // fires on a misconfiguration (e.g. operator switched from a
            // raw key to a passphrase without re-sealing) — surface it
            // clearly rather than feeding a mismatched key to the AEAD
            // (which would just fail the tag check with a vaguer error).
            (Self::RawKey(_), KDF_ARGON2ID) => Err(VaultError::Invalid(
                "vault KEK was sealed with a passphrase (Argon2id) but the configured unseal \
                 material is a raw key. Provide the original passphrase, or re-seal."
                    .into(),
            )),
            (Self::Passphrase(_), KDF_RAW) => Err(VaultError::Invalid(
                "vault KEK was sealed with a raw key but the configured unseal material is a \
                 passphrase. Provide the original raw key, or re-seal."
                    .into(),
            )),
            (_, other) => Err(VaultError::Invalid(format!(
                "unknown KEK seal KDF tag {other}; this build understands raw (0x00) + \
                 argon2id (0x01)"
            ))),
        }
    }

    /// KDF tag this material seals under for a fresh seal.
    fn fresh_kdf(&self) -> u8 {
        match self {
            Self::RawKey(_) => KDF_RAW,
            Self::Passphrase(_) => KDF_ARGON2ID,
        }
    }
}

/// How the engine should obtain the unseal material. Parsed from the
/// `vault.unseal_key_source` config string (mirrors the repo's
/// `${VAR}`-style source-string convention in `engine.toml`).
///
/// Supported source strings:
/// - `env:NAME` — read base64 raw key (or passphrase, if `passphrase:`
///   prefixed inside the var) from environment variable `NAME`.
/// - `file:/path` — read the material from a file. The file MUST be
///   `0600` (owner-read/write only) or boot fails; a world-readable
///   unseal key is no better than a plaintext KEK.
/// - `base64:BBBB` — an inline base64 raw key. Convenient for dev /
///   tests; in production prefer `env:`/`file:` so the key isn't in the
///   config file.
/// - `passphrase:TEXT` — an inline passphrase (Argon2id-stretched).
///
/// `env:`/`file:` values may themselves be prefixed with `passphrase:`
/// to opt into the Argon2id path instead of base64-raw-key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnsealSource {
    Env(String),
    File(std::path::PathBuf),
    InlineBase64(String),
    InlinePassphrase(String),
}

impl UnsealSource {
    /// Parse a `vault.unseal_key_source` string. Returns `None` for an
    /// empty/whitespace string (i.e. "no source configured") so the
    /// caller can decide whether that's a fail-closed condition.
    pub fn parse(s: &str) -> Result<Option<Self>> {
        let s = s.trim();
        if s.is_empty() {
            return Ok(None);
        }
        let src = if let Some(v) = s.strip_prefix("env:") {
            UnsealSource::Env(v.trim().to_string())
        } else if let Some(v) = s.strip_prefix("file:") {
            UnsealSource::File(std::path::PathBuf::from(v.trim()))
        } else if let Some(v) = s.strip_prefix("base64:") {
            UnsealSource::InlineBase64(v.trim().to_string())
        } else if let Some(v) = s.strip_prefix("passphrase:") {
            UnsealSource::InlinePassphrase(v.to_string())
        } else {
            return Err(VaultError::Invalid(format!(
                "vault.unseal_key_source '{s}' has no recognized prefix. Use one of \
                 `env:NAME`, `file:/path`, `base64:...`, or `passphrase:...`."
            )));
        };
        Ok(Some(src))
    }

    /// Resolve the source into concrete [`UnsealMaterial`]. Reads the
    /// env var / file and applies the 0600 permission check on files.
    /// A leading `passphrase:` inside the resolved value selects the
    /// Argon2id path; otherwise the value is treated as a base64 raw key.
    pub fn resolve(&self) -> Result<UnsealMaterial> {
        match self {
            UnsealSource::Env(name) => {
                let val = std::env::var(name).map_err(|_| {
                    VaultError::Invalid(format!(
                        "vault unseal env var '{name}' is not set. Set it to a base64 32-byte key \
                         (e.g. `export {name}=$(openssl rand -base64 32)`) or a \
                         `passphrase:<text>` value, then reboot."
                    ))
                })?;
                material_from_value(&val).map_err(|e| annotate(e, &format!("env var '{name}'")))
            }
            UnsealSource::File(path) => {
                let val = read_unseal_file(path)?;
                material_from_value(val.trim())
                    .map_err(|e| annotate(e, &format!("file '{}'", path.display())))
            }
            UnsealSource::InlineBase64(b64) => UnsealMaterial::raw_key_from_base64(b64),
            UnsealSource::InlinePassphrase(pw) => UnsealMaterial::passphrase(pw.clone()),
        }
    }
}

/// Interpret a resolved unseal-source value: a leading `passphrase:`
/// selects Argon2id; everything else is a base64 raw key.
fn material_from_value(val: &str) -> Result<UnsealMaterial> {
    if let Some(pw) = val.strip_prefix("passphrase:") {
        UnsealMaterial::passphrase(pw)
    } else {
        UnsealMaterial::raw_key_from_base64(val)
    }
}

fn annotate(e: VaultError, src: &str) -> VaultError {
    match e {
        VaultError::Invalid(m) => VaultError::Invalid(format!("{m} (source: {src})")),
        other => other,
    }
}

/// Read an unseal-key file, enforcing `0600` perms on Unix. A
/// world/group-readable unseal key would defeat the purpose, so we
/// fail closed rather than warn.
fn read_unseal_file(path: &std::path::Path) -> Result<Zeroizing<String>> {
    let meta = std::fs::metadata(path).map_err(|e| {
        VaultError::Invalid(format!(
            "vault unseal key file '{}' could not be read: {e}",
            path.display()
        ))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(VaultError::Invalid(format!(
                "vault unseal key file '{}' has permissions {:o}; it must be 0600 \
                 (owner read/write only). Run `chmod 600 {}` and reboot.",
                path.display(),
                mode,
                path.display()
            )));
        }
    }
    let s = std::fs::read_to_string(path).map_err(|e| {
        VaultError::Invalid(format!(
            "vault unseal key file '{}' could not be read: {e}",
            path.display()
        ))
    })?;
    Ok(Zeroizing::new(s))
}

/// Seal a 32-byte KEK under `material`, producing the `sealed_blob`
/// bytes for `vault.kek_metadata`. The plaintext KEK is never written;
/// only this blob is persisted.
pub fn seal_kek(kek: &[u8; KEY_LEN], kid: &str, material: &UnsealMaterial) -> Result<Vec<u8>> {
    let kdf = material.fresh_kdf();
    let salt: Vec<u8> = match kdf {
        KDF_ARGON2ID => {
            let mut s = vec![0u8; ARGON2_SALT_LEN];
            rand::rng().fill_bytes(&mut s);
            s
        }
        _ => Vec::new(),
    };
    if salt.len() > u8::MAX as usize {
        return Err(VaultError::Crypto("seal salt too long".into()));
    }
    let wrapping = material.wrapping_key(kdf, &salt)?;
    let nonce = random_nonce();
    let aad = seal_aad(kid);
    let ct = encrypt(&wrapping, &nonce, &aad, kek)?;

    let mut blob = Vec::with_capacity(3 + salt.len() + NONCE_LEN + ct.len());
    blob.push(SEAL_BLOB_VERSION);
    blob.push(kdf);
    blob.push(salt.len() as u8);
    blob.extend_from_slice(&salt);
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ct);
    Ok(blob)
}

/// Unseal a `sealed_blob` produced by [`seal_kek`], returning the raw
/// 32-byte KEK. Fails (no fallback) if:
/// - the blob version/format is unrecognized,
/// - the material kind doesn't match the blob's KDF tag,
/// - the AEAD tag doesn't validate (wrong key/passphrase, tampered blob,
///   or wrong `kid`).
pub fn unseal_kek(blob: &[u8], kid: &str, material: &UnsealMaterial) -> Result<[u8; KEY_LEN]> {
    // version(1) + kdf(1) + salt_len(1) + nonce(12) + ciphertext(>=KEY_LEN+16)
    const MIN_LEN: usize = 3 + NONCE_LEN + KEY_LEN + 16;
    if blob.len() < MIN_LEN {
        return Err(VaultError::Crypto(format!(
            "sealed KEK blob is {} bytes; too short to be a valid sealed-v1 blob (min {MIN_LEN})",
            blob.len()
        )));
    }
    let version = blob[0];
    if version != SEAL_BLOB_VERSION {
        return Err(VaultError::Invalid(format!(
            "sealed KEK blob version {version} is not supported by this build (expects \
             {SEAL_BLOB_VERSION}). A newer engine wrote this row; upgrade the engine."
        )));
    }
    let kdf = blob[1];
    let salt_len = blob[2] as usize;
    let body = &blob[3..];
    if body.len() < salt_len + NONCE_LEN + KEY_LEN + 16 {
        return Err(VaultError::Crypto(
            "sealed KEK blob truncated: salt_len exceeds remaining bytes".into(),
        ));
    }
    let salt = &body[..salt_len];
    let rest = &body[salt_len..];
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&rest[..NONCE_LEN]);
    let ct = &rest[NONCE_LEN..];

    let wrapping = material.wrapping_key(kdf, salt)?;
    let aad = seal_aad(kid);
    let pt = decrypt(&wrapping, &nonce, &aad, ct).map_err(|_| {
        VaultError::Crypto(
            "vault KEK unseal failed: the auth tag did not validate. The unseal key/passphrase is \
             wrong, the sealed blob was tampered with, or it belongs to a different KEK. The \
             engine will NOT fall back to plaintext."
                .into(),
        )
    })?;
    if pt.len() != KEY_LEN {
        return Err(VaultError::Crypto(format!(
            "unsealed KEK is {} bytes; expected {KEY_LEN}",
            pt.len()
        )));
    }
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&pt);
    Ok(key)
}

fn seal_aad(kid: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(SEAL_AAD_PREFIX.len() + kid.len());
    aad.extend_from_slice(SEAL_AAD_PREFIX);
    aad.extend_from_slice(kid.as_bytes());
    aad
}

/// Decode base64 in whichever common alphabet the operator pasted —
/// standard or URL-safe, padded or not. Returns `None` on failure.
fn decode_base64_any(s: &str) -> Option<Vec<u8>> {
    use data_encoding::{BASE64, BASE64URL, BASE64URL_NOPAD, BASE64_NOPAD};
    for enc in [&BASE64, &BASE64_NOPAD, &BASE64URL, &BASE64URL_NOPAD] {
        if let Ok(v) = enc.decode(s.as_bytes()) {
            return Some(v);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::aead::random_dek;

    fn raw_material() -> UnsealMaterial {
        let key = random_dek();
        UnsealMaterial::RawKey(Zeroizing::new(key))
    }

    #[test]
    fn raw_key_seal_unseal_round_trip() {
        let kek = random_dek();
        let m = raw_material();
        let blob = seal_kek(&kek, "kek-abc", &m).unwrap();
        // The plaintext KEK must not appear verbatim in the blob.
        assert!(
            blob.windows(KEY_LEN).all(|w| w != kek),
            "sealed blob must not contain the raw KEK"
        );
        let recovered = unseal_kek(&blob, "kek-abc", &m).unwrap();
        assert_eq!(recovered, kek);
    }

    #[test]
    fn passphrase_seal_unseal_round_trip() {
        let kek = random_dek();
        let m = UnsealMaterial::passphrase("correct horse battery staple").unwrap();
        let blob = seal_kek(&kek, "kek-pw", &m).unwrap();
        assert_eq!(blob[1], KDF_ARGON2ID, "passphrase seal must tag argon2id");
        let m2 = UnsealMaterial::passphrase("correct horse battery staple").unwrap();
        let recovered = unseal_kek(&blob, "kek-pw", &m2).unwrap();
        assert_eq!(recovered, kek);
    }

    #[test]
    fn wrong_raw_key_fails_tag() {
        let kek = random_dek();
        let m = raw_material();
        let blob = seal_kek(&kek, "kek-1", &m).unwrap();
        let wrong = raw_material();
        let res = unseal_kek(&blob, "kek-1", &wrong);
        assert!(matches!(res, Err(VaultError::Crypto(_))), "wrong key must fail");
    }

    #[test]
    fn wrong_passphrase_fails_tag() {
        let kek = random_dek();
        let m = UnsealMaterial::passphrase("right-passphrase").unwrap();
        let blob = seal_kek(&kek, "kek-1", &m).unwrap();
        let wrong = UnsealMaterial::passphrase("wrong-passphrase").unwrap();
        assert!(unseal_kek(&blob, "kek-1", &wrong).is_err());
    }

    #[test]
    fn wrong_kid_fails_tag() {
        let kek = random_dek();
        let m = raw_material();
        let blob = seal_kek(&kek, "kek-correct", &m).unwrap();
        // Same material, different kid → AAD mismatch → tag fails.
        assert!(unseal_kek(&blob, "kek-other", &m).is_err());
    }

    #[test]
    fn tampered_blob_fails() {
        let kek = random_dek();
        let m = raw_material();
        let mut blob = seal_kek(&kek, "kek-t", &m).unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0xff;
        assert!(unseal_kek(&blob, "kek-t", &m).is_err());
    }

    #[test]
    fn material_kind_mismatch_is_actionable() {
        let kek = random_dek();
        let m = raw_material();
        let blob = seal_kek(&kek, "kek-x", &m).unwrap();
        let pw = UnsealMaterial::passphrase("pw").unwrap();
        let err = unseal_kek(&blob, "kek-x", &pw).unwrap_err();
        assert!(
            matches!(err, VaultError::Invalid(_)),
            "raw-sealed blob unsealed with a passphrase should be an actionable Invalid, got {err:?}"
        );
    }

    #[test]
    fn truncated_blob_rejected() {
        let m = raw_material();
        assert!(unseal_kek(&[1u8, 2, 3], "kid", &m).is_err());
    }

    #[test]
    fn future_version_rejected() {
        let kek = random_dek();
        let m = raw_material();
        let mut blob = seal_kek(&kek, "kek-v", &m).unwrap();
        blob[0] = 99;
        let err = unseal_kek(&blob, "kek-v", &m).unwrap_err();
        assert!(matches!(err, VaultError::Invalid(_)));
    }

    #[test]
    fn raw_key_from_base64_validates_length() {
        // 32 bytes → ok.
        let good = data_encoding::BASE64.encode(&[7u8; KEY_LEN]);
        assert!(UnsealMaterial::raw_key_from_base64(&good).is_ok());
        // 16 bytes → rejected with actionable error.
        let short = data_encoding::BASE64.encode(&[7u8; 16]);
        let err = UnsealMaterial::raw_key_from_base64(&short).unwrap_err();
        assert!(matches!(err, VaultError::Invalid(_)));
        // Not base64 at all.
        assert!(UnsealMaterial::raw_key_from_base64("not base64!!!").is_err());
    }

    #[test]
    fn raw_key_from_base64_accepts_urlsafe_and_nopad() {
        let bytes = [3u8; KEY_LEN];
        for enc in [
            &data_encoding::BASE64,
            &data_encoding::BASE64_NOPAD,
            &data_encoding::BASE64URL,
            &data_encoding::BASE64URL_NOPAD,
        ] {
            let s = enc.encode(&bytes);
            assert!(
                UnsealMaterial::raw_key_from_base64(&s).is_ok(),
                "should accept {s}"
            );
        }
    }

    #[test]
    fn empty_passphrase_rejected() {
        assert!(UnsealMaterial::passphrase("   ").is_err());
    }

    #[test]
    fn source_parse_recognizes_prefixes() {
        assert_eq!(
            UnsealSource::parse("env:FOO").unwrap(),
            Some(UnsealSource::Env("FOO".into()))
        );
        assert_eq!(
            UnsealSource::parse("file:/etc/key").unwrap(),
            Some(UnsealSource::File("/etc/key".into()))
        );
        assert_eq!(
            UnsealSource::parse("base64:AAAA").unwrap(),
            Some(UnsealSource::InlineBase64("AAAA".into()))
        );
        assert_eq!(
            UnsealSource::parse("passphrase:hunter2").unwrap(),
            Some(UnsealSource::InlinePassphrase("hunter2".into()))
        );
        // Empty → no source configured.
        assert_eq!(UnsealSource::parse("   ").unwrap(), None);
        // Unknown prefix → actionable error.
        assert!(UnsealSource::parse("vault:secret").is_err());
    }

    #[test]
    fn source_env_resolves_base64_and_passphrase() {
        let key_b64 = data_encoding::BASE64.encode(&[9u8; KEY_LEN]);
        // SAFETY: single-threaded test; var name is unique per test.
        unsafe {
            std::env::set_var("ASSAY_TEST_UNSEAL_RAW", &key_b64);
            std::env::set_var("ASSAY_TEST_UNSEAL_PW", "passphrase:my secret");
        }
        let m = UnsealSource::Env("ASSAY_TEST_UNSEAL_RAW".into())
            .resolve()
            .unwrap();
        assert!(matches!(m, UnsealMaterial::RawKey(_)));
        let m2 = UnsealSource::Env("ASSAY_TEST_UNSEAL_PW".into())
            .resolve()
            .unwrap();
        assert!(matches!(m2, UnsealMaterial::Passphrase(_)));
        unsafe {
            std::env::remove_var("ASSAY_TEST_UNSEAL_RAW");
            std::env::remove_var("ASSAY_TEST_UNSEAL_PW");
        }
    }

    #[test]
    fn source_env_missing_is_actionable() {
        let err = UnsealSource::Env("ASSAY_DEFINITELY_UNSET_VAR_XYZ".into())
            .resolve()
            .unwrap_err();
        assert!(matches!(err, VaultError::Invalid(_)));
    }

    #[cfg(unix)]
    #[test]
    fn source_file_rejects_loose_perms() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir();
        let path = dir.join(format!("assay-unseal-perm-{}.key", std::process::id()));
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "{}", data_encoding::BASE64.encode(&[1u8; KEY_LEN])).unwrap();
        drop(f);
        // 0644 → must be rejected.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = UnsealSource::File(path.clone()).resolve().unwrap_err();
        assert!(matches!(err, VaultError::Invalid(_)));
        // 0600 → accepted.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(UnsealSource::File(path.clone()).resolve().is_ok());
        let _ = std::fs::remove_file(&path);
    }
}
