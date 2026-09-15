//! Where the unseal material comes from.
//!
//! [`crate::crypto::env_seal`] owns the crypto; this module owns the
//! question of which string to hand it. A secret in the process
//! environment is readable from `/proc/<pid>/environ`, is carried into
//! core dumps, and shows up in `systemctl show -p Environment`, so a
//! deployment that keeps its secrets in files — or derives them from an
//! operator passphrase — needs somewhere else to put one.
//!
//! ## The same string is the same key
//!
//! Every source except [`SealSource::Passphrase`] resolves to a trimmed
//! string and goes through [`SealKey::derive_from`], which is the same
//! SHA-256-over-a-label the environment variable has always used. So the
//! identical value works whether it arrives by variable, by file, or
//! inline, and an operator can move a secret between them without
//! re-sealing the store.
//!
//! That is also why [`SealSource::Value`] does **not** base64-decode
//! what it is given, even though an inline key is usually base64.
//! Deployments already put base64 in the variable and it gets hashed;
//! a source that decoded instead would derive a *different* key from
//! the same string, so an operator moving `ASSAY_VAULT_SEAL_KEY` into
//! `[vault.sealing] value` would be told their correct secret "does not
//! decrypt". `Value` means "any string, base64 included, derived exactly
//! as the variable is".
//!
//! ## Absence versus failure
//!
//! Only [`SealSource::Env`] may resolve to `Ok(None)`, and only when the
//! variable is unset or empty — the historical shape, and the one case
//! where "no material" is a plausible steady state rather than a
//! mistake. Every explicitly-configured source that cannot produce
//! material is an error: a `file` source pointed at a path that does not
//! exist is a broken deployment, not an unsealed one.

use std::io::Read;
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::crypto::env_seal::SealKey;
use crate::error::{Result, VaultError};

/// Config fields named in operator-facing errors, so the text an
/// operator reads and the line they have to edit cannot drift apart.
pub const CFG_VALUE: &str = "[vault.sealing] value";
pub const CFG_PASSPHRASE: &str = "[vault.sealing] passphrase";
pub const CFG_SALT: &str = "[vault.sealing] salt";

/// Shortest accepted salt for the passphrase source. Argon2 itself
/// accepts 8 bytes; 16 is the floor worth enforcing.
pub const MIN_SALT_CHARS: usize = 16;

/// Longest salt Argon2 accepts.
pub const MAX_SALT_CHARS: usize = 64;

/// Cap on how much of a seal key file is read, so a path that points at
/// a device or a huge file fails instead of consuming memory. No real
/// seal key is anywhere near this.
const MAX_KEY_FILE_BYTES: u64 = 64 * 1024;

/// Argon2id cost parameters for [`SealSource::Passphrase`] — the OWASP
/// second-choice profile (19 MiB, 2 passes, 1 lane). They are fixed
/// rather than configurable: the derived key depends on them, so an
/// operator who tuned them would silently lock themselves out of the
/// store on the next restart.
#[cfg(feature = "vault-sealing-passphrase")]
const ARGON2_M_COST: u32 = 19_456;
#[cfg(feature = "vault-sealing-passphrase")]
const ARGON2_T_COST: u32 = 2;
#[cfg(feature = "vault-sealing-passphrase")]
const ARGON2_P_COST: u32 = 1;

/// A configured place to read unseal material from.
#[non_exhaustive]
#[derive(Clone)]
pub enum SealSource {
    /// An environment variable, named by the deployment. The historical
    /// source, and the only one that may be absent without failing.
    Env { var: String },
    /// A file, whose permissions are checked before it is read.
    File { path: PathBuf },
    /// The key inline in the config — which in practice means a `${VAR}`
    /// reference the config loader already expanded.
    Value { value: Zeroizing<String> },
    /// A human passphrase run through Argon2id with a fixed, public
    /// salt. Requires the `vault-sealing-passphrase` feature.
    Passphrase {
        passphrase: Zeroizing<String>,
        /// Public, and required: the derived key has to survive a
        /// restart, so the salt cannot be random per-boot. Its job is
        /// separation between deployments, not secrecy.
        salt: String,
    },
}

/// Hand-written: a derived `Debug` would print the inline value and the
/// passphrase verbatim, and this type ends up inside a config struct
/// that any log line might render.
impl std::fmt::Debug for SealSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Env { var } => write!(f, "SealSource::Env {{ var: {var:?} }}"),
            Self::File { path } => write!(f, "SealSource::File {{ path: {:?} }}", path.display()),
            Self::Value { .. } => f.write_str("SealSource::Value { value: redacted }"),
            Self::Passphrase { salt, .. } => write!(
                f,
                "SealSource::Passphrase {{ passphrase: redacted, salt: {salt:?} }}"
            ),
        }
    }
}

impl SealSource {
    /// The environment source with the default variable name.
    pub fn default_env() -> Self {
        Self::Env {
            var: crate::crypto::env_seal::ENV_VAR.to_string(),
        }
    }

    /// Resolve this source to a seal key.
    ///
    /// `Ok(None)` means "there is genuinely no material here", which
    /// only [`Self::Env`] can report. Anything else that cannot produce
    /// a key is an error naming what to go and fix.
    pub fn resolve(&self) -> Result<Option<SealKey>> {
        let key = match self {
            Self::Env { var } => return resolve_env(var),
            Self::File { path } => read_key_file(path)?,
            Self::Value { value } => SealKey::derive_from(value, CFG_VALUE)?,
            Self::Passphrase { passphrase, salt } => derive_from_passphrase(passphrase, salt)?,
        };
        Ok(Some(key))
    }

    /// The variable this source reads, if it reads one. Used to tell an
    /// operator who renamed it which name to actually set.
    pub fn env_var(&self) -> Option<&str> {
        match self {
            Self::Env { var } => Some(var),
            _ => None,
        }
    }
}

/// The environment is the one source allowed to be silently absent, so
/// it gets its own function rather than an arm that has to `return`
/// something shaped differently from every other arm.
fn resolve_env(var: &str) -> Result<Option<SealKey>> {
    match std::env::var(var) {
        Ok(raw) if !raw.trim().is_empty() => SealKey::derive_from(&raw, var).map(Some),
        _ => Ok(None),
    }
}

/// Read a seal key out of a file, refusing one anybody else can read.
fn read_key_file(path: &Path) -> Result<SealKey> {
    let file = std::fs::File::open(path).map_err(|e| {
        VaultError::Invalid(format!(
            "open the seal key file {}: {e}; `[vault.sealing] source = \"file\"` names it, \
             so an unreadable path is a boot failure rather than an unsealed vault",
            path.display()
        ))
    })?;
    // Checked against the open handle rather than the path, so a symlink
    // swapped between the check and the read cannot win the race.
    check_key_file_permissions(&file, path)?;

    // One byte past the cap, so an oversized file is refused rather than
    // silently truncated: deriving a key from the first 64 KiB of the
    // wrong file would seal the store under something nobody can
    // reproduce, and it would look like it had worked.
    let mut raw = Zeroizing::new(String::new());
    let read = file
        .take(MAX_KEY_FILE_BYTES + 1)
        .read_to_string(&mut raw)
        .map_err(|e| {
            VaultError::Invalid(format!("read the seal key file {}: {e}", path.display()))
        })?;
    if read as u64 > MAX_KEY_FILE_BYTES {
        return Err(VaultError::Invalid(format!(
            "the seal key file {} is larger than {MAX_KEY_FILE_BYTES} bytes, so it is not a \
             seal key; check the path",
            path.display()
        )));
    }

    SealKey::derive_from(&raw, format!("the seal key file {}", path.display()))
}

#[cfg(unix)]
fn check_key_file_permissions(file: &std::fs::File, path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let meta = file.metadata().map_err(|e| {
        VaultError::Invalid(format!("stat the seal key file {}: {e}", path.display()))
    })?;
    let mode = meta.mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(VaultError::Invalid(format!(
            "the seal key file {} is mode {mode:04o}, which lets group or other read it; \
             `chmod 600` it and restart",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_key_file_permissions(_file: &std::fs::File, path: &Path) -> Result<()> {
    tracing::warn!(
        target: "assay-vault",
        path = %path.display(),
        "seal key file permissions are not checked on this platform; \
         make sure only the account the engine runs as can read it"
    );
    Ok(())
}

#[cfg(feature = "vault-sealing-passphrase")]
fn derive_from_passphrase(passphrase: &str, salt: &str) -> Result<SealKey> {
    use crate::crypto::aead::KEY_LEN;
    use crate::crypto::env_seal::MIN_CHARS;
    use zeroize::Zeroize;

    let phrase = passphrase.trim();
    let length = phrase.chars().count();
    if length < MIN_CHARS {
        return Err(VaultError::Invalid(format!(
            "{CFG_PASSPHRASE} must be at least {MIN_CHARS} characters, got {length}"
        )));
    }
    let salt_len = salt.chars().count();
    if !(MIN_SALT_CHARS..=MAX_SALT_CHARS).contains(&salt_len) {
        return Err(VaultError::Invalid(format!(
            "{CFG_SALT} must be {MIN_SALT_CHARS}..={MAX_SALT_CHARS} characters, got {salt_len}"
        )));
    }

    let params = argon2::Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(KEY_LEN))
        .map_err(|e| VaultError::Invalid(format!("argon2 parameters: {e}")))?;
    let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; KEY_LEN];
    argon
        .hash_password_into(phrase.as_bytes(), salt.as_bytes(), &mut key)
        .map_err(|e| {
            VaultError::Crypto(format!("derive the seal key from {CFG_PASSPHRASE}: {e}"))
        })?;
    // Argon2's output is already a uniform 32 bytes, so it goes in as
    // the key rather than back through the SHA-256 the other sources
    // use — hashing it again would only hide the work.
    let sealed = SealKey::from_key_bytes(key, CFG_PASSPHRASE);
    key.zeroize();
    Ok(sealed)
}

#[cfg(not(feature = "vault-sealing-passphrase"))]
fn derive_from_passphrase(_passphrase: &str, _salt: &str) -> Result<SealKey> {
    Err(VaultError::Invalid(
        "[vault.sealing] source = \"passphrase\" needs the `vault-sealing-passphrase` feature, \
         which this binary was built without; use a file, a value, or the environment variable"
            .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = "YzBmZTFhMmIzYzRkNWU2ZjcwODE5MmEzYjRjNWQ2ZTc=";

    fn write_key_file(dir: &tempfile::TempDir, name: &str, body: &str, mode: u32) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, body).expect("write key file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                .expect("chmod key file");
        }
        let _ = mode;
        path
    }

    fn key_from_file(path: PathBuf) -> SealKey {
        SealSource::File { path }.resolve().unwrap().unwrap()
    }

    /// The point of the whole resolution layer: a value keeps its
    /// meaning when an operator moves it between sources.
    #[test]
    fn a_file_and_the_environment_derive_the_same_key_from_the_same_value() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_key_file(&dir, "seal", RAW, 0o600);

        let from_file = key_from_file(path);
        let from_value = SealSource::Value {
            value: Zeroizing::new(RAW.to_string()),
        }
        .resolve()
        .unwrap()
        .unwrap();

        let blob = from_file.seal("kek-abc", &[8u8; 32]).unwrap();
        assert_eq!(from_value.unseal("kek-abc", &blob).unwrap(), [8u8; 32]);
    }

    /// A secret delivered as a file arrives with a trailing newline.
    #[test]
    fn a_trailing_newline_in_the_file_does_not_change_the_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let padded = write_key_file(&dir, "padded", &format!("{RAW}\n"), 0o600);
        let exact = write_key_file(&dir, "exact", RAW, 0o600);

        let a = key_from_file(padded);
        let b = key_from_file(exact);
        let blob = a.seal("kek-abc", &[1u8; 32]).unwrap();
        assert_eq!(b.unseal("kek-abc", &blob).unwrap(), [1u8; 32]);
    }

    /// A path pointing at the wrong thing must fail, not seal the store
    /// under a key derived from the first 64 KiB of it.
    #[test]
    fn an_oversized_seal_key_file_is_refused_rather_than_truncated() {
        let dir = tempfile::tempdir().expect("tempdir");
        let big = "a".repeat(MAX_KEY_FILE_BYTES as usize + 1);
        let path = write_key_file(&dir, "huge", &big, 0o600);

        let err = SealSource::File { path }.resolve().unwrap_err();
        assert!(err.to_string().contains("larger than"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn a_seal_key_file_anybody_can_read_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_key_file(&dir, "loose", RAW, 0o644);

        let err = SealSource::File { path: path.clone() }
            .resolve()
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(&path.display().to_string()), "{msg}");
        assert!(msg.contains("0644"), "{msg}");
    }

    /// A configured source that cannot produce material is a broken
    /// deployment, not an unsealed one.
    #[test]
    fn a_missing_seal_key_file_is_an_error_rather_than_no_material() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = SealSource::File {
            path: dir.path().join("absent"),
        }
        .resolve()
        .unwrap_err();
        assert!(err.to_string().contains("absent"), "{err}");
    }

    #[test]
    fn a_short_value_is_refused_and_the_message_names_the_field() {
        let err = SealSource::Value {
            value: Zeroizing::new("too-short".to_string()),
        }
        .resolve()
        .unwrap_err();
        assert!(err.to_string().contains("[vault.sealing] value"), "{err}");
    }

    /// Only the environment may be silently absent.
    #[test]
    fn an_unset_environment_variable_resolves_to_no_material() {
        let source = SealSource::Env {
            var: "ASSAY_VAULT_SEAL_KEY_DEFINITELY_UNSET_IN_TESTS".to_string(),
        };
        assert!(source.resolve().unwrap().is_none());
    }

    /// The inline value and the passphrase must not reach a log line
    /// because something upstream derived `Debug`.
    #[test]
    fn debug_redacts_the_secret_bearing_sources() {
        let value = format!(
            "{:?}",
            SealSource::Value {
                value: Zeroizing::new(RAW.to_string()),
            }
        );
        assert!(value.contains("redacted"), "{value}");
        assert!(!value.contains(RAW), "{value}");

        let phrase = format!(
            "{:?}",
            SealSource::Passphrase {
                passphrase: Zeroizing::new("correct horse battery staple".to_string()),
                salt: "a-public-salt-value".to_string(),
            }
        );
        assert!(phrase.contains("redacted"), "{phrase}");
        assert!(!phrase.contains("correct horse"), "{phrase}");
        // The salt is public and worth showing — it is what an operator
        // has to match to reopen the store.
        assert!(phrase.contains("a-public-salt-value"), "{phrase}");
    }

    #[cfg(feature = "vault-sealing-passphrase")]
    mod passphrase {
        use super::*;

        const PHRASE: &str = "correct horse battery staple correct horse";
        const SALT: &str = "assay-vault-test-salt";

        fn source(phrase: &str, salt: &str) -> SealSource {
            SealSource::Passphrase {
                passphrase: Zeroizing::new(phrase.to_string()),
                salt: salt.to_string(),
            }
        }

        /// The KEK has to reopen after a restart, so the derivation has
        /// to be a pure function of the passphrase and the salt.
        #[test]
        fn the_same_passphrase_and_salt_reopen_the_store() {
            let a = source(PHRASE, SALT).resolve().unwrap().unwrap();
            let b = source(PHRASE, SALT).resolve().unwrap().unwrap();
            let blob = a.seal("kek-abc", &[4u8; 32]).unwrap();
            assert_eq!(b.unseal("kek-abc", &blob).unwrap(), [4u8; 32]);
        }

        #[test]
        fn a_different_salt_opens_nothing() {
            let a = source(PHRASE, SALT).resolve().unwrap().unwrap();
            let b = source(PHRASE, "a-different-public-salt")
                .resolve()
                .unwrap()
                .unwrap();
            let blob = a.seal("kek-abc", &[4u8; 32]).unwrap();
            assert!(b.unseal("kek-abc", &blob).is_err());
        }

        /// The passphrase path is the one that does not go through
        /// SHA-256, so it must not land on the same key as the value
        /// path for the same string.
        #[test]
        fn a_passphrase_is_not_the_same_key_as_the_same_string_inline() {
            let kdf = source(PHRASE, SALT).resolve().unwrap().unwrap();
            let plain = SealSource::Value {
                value: Zeroizing::new(PHRASE.to_string()),
            }
            .resolve()
            .unwrap()
            .unwrap();
            let blob = kdf.seal("kek-abc", &[4u8; 32]).unwrap();
            assert!(plain.unseal("kek-abc", &blob).is_err());
        }

        #[test]
        fn a_salt_below_the_floor_is_refused_and_named() {
            let err = source(PHRASE, "short").resolve().unwrap_err();
            let msg = err.to_string();
            assert!(msg.contains("[vault.sealing] salt"), "{msg}");
            assert!(msg.contains(&MIN_SALT_CHARS.to_string()), "{msg}");
        }

        #[test]
        fn a_salt_above_the_ceiling_is_refused() {
            let err = source(PHRASE, &"s".repeat(MAX_SALT_CHARS + 1))
                .resolve()
                .unwrap_err();
            assert!(err.to_string().contains(CFG_SALT), "{err}");

            let at_ceiling = source(PHRASE, &"s".repeat(MAX_SALT_CHARS));
            assert!(
                at_ceiling.resolve().is_ok(),
                "exactly the ceiling must be accepted"
            );
        }
    }
}
