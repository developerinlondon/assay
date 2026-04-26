//! Argon2id password hashing wrapper.
//!
//! Plan 11 reference: Argon2id with sensible defaults. We pick
//! m=64 MiB, t=3, p=4 — comfortably above the OWASP 2024 minimum
//! (m=19 MiB, t=2) and within reach of a small worker process. Tunable
//! later via [`PasswordHasher::with_params`] when a deployment needs to
//! shift the cost to match its hardware envelope.
//!
//! Hashes are stored as PHC-format strings (`$argon2id$v=19$...`) on
//! `auth.users.password_hash`; the algorithm and parameters travel with
//! the hash, so [`PasswordHasher::needs_rehash`] can detect drift across
//! deployments and trigger an opportunistic re-hash on next successful
//! login.

use argon2::{Algorithm, Argon2, Params, Version};
use password_hash::{Salt, PasswordHash, PasswordHasher as PhcHasher, PasswordVerifier, SaltString};
use rand::RngCore;

use crate::error::{Error, Result};

/// Memory cost in KiB. 65 536 KiB = 64 MiB. Above OWASP 2024 minimum.
const DEFAULT_MEMORY_KIB: u32 = 65_536;
/// Time cost (number of passes). Three is a balance between
/// brute-force resistance and login latency on commodity hardware.
const DEFAULT_TIME_COST: u32 = 3;
/// Parallelism. Four threads matches the typical small-server CPU
/// shape; the implementation is internally parallel on supported
/// platforms.
const DEFAULT_PARALLELISM: u32 = 4;

/// Stateless hasher — owns one [`Argon2`] context preconfigured with
/// the active parameters. Cheap to clone (the inner `Argon2<'static>`
/// holds borrowed references to the static parameter struct only).
#[derive(Clone)]
pub struct PasswordHasher {
    argon: Argon2<'static>,
    params: Params,
}

impl Default for PasswordHasher {
    fn default() -> Self {
        let params = Params::new(
            DEFAULT_MEMORY_KIB,
            DEFAULT_TIME_COST,
            DEFAULT_PARALLELISM,
            None,
        )
        .expect("argon2 default params are within library limits");
        Self {
            argon: Argon2::new(Algorithm::Argon2id, Version::V0x13, params.clone()),
            params,
        }
    }
}

impl PasswordHasher {
    /// Construct with explicit Argon2id parameters. Use the [`Default`]
    /// impl for the standard cost; reach for this only when tuning to
    /// non-standard hardware (e.g. a mobile-only deployment).
    pub fn with_params(params: Params) -> Self {
        Self {
            argon: Argon2::new(Algorithm::Argon2id, Version::V0x13, params.clone()),
            params,
        }
    }

    /// Hash a plaintext password. The returned PHC string is what
    /// [`crate::store::UserStore::set_password_hash`] persists.
    ///
    /// The salt is sourced from `rand::rng()` (which delegates to the
    /// OS getrandom under the hood) so we don't need to depend on
    /// `password-hash`'s rand_core 0.6 + `getrandom` feature pair just
    /// for the salt — `rand` 0.9 is already a direct dependency.
    pub fn hash(&self, plaintext: &str) -> Result<String> {
        let mut salt_bytes = [0u8; Salt::RECOMMENDED_LENGTH];
        rand::rng().fill_bytes(&mut salt_bytes);
        let salt = SaltString::encode_b64(&salt_bytes).map_err(map_phc_err)?;
        let phc = self
            .argon
            .hash_password(plaintext.as_bytes(), &salt)
            .map_err(map_phc_err)?;
        Ok(phc.to_string())
    }

    /// Verify a plaintext password against a stored PHC hash.
    ///
    /// Returns:
    /// - `Ok(true)` if the password matches.
    /// - `Ok(false)` if the password is wrong (but the stored hash
    ///   parsed cleanly).
    /// - `Err(Error::Backend(_))` if the stored hash is malformed.
    pub fn verify(&self, plaintext: &str, phc: &str) -> Result<bool> {
        let parsed = PasswordHash::new(phc).map_err(map_phc_err)?;
        match self.argon.verify_password(plaintext.as_bytes(), &parsed) {
            Ok(()) => Ok(true),
            // `Password` variant means parsing succeeded but the digest
            // doesn't match — that's a "wrong password", not a system
            // error. Anything else (e.g. unsupported algorithm) is real.
            Err(password_hash::Error::Password) => Ok(false),
            Err(other) => Err(map_phc_err(other)),
        }
    }

    /// Returns `true` when the stored hash was produced with parameters
    /// that differ from the current configuration. Callers should
    /// re-hash on the next successful login so the user's stored hash
    /// drifts forward as we ratchet the cost.
    pub fn needs_rehash(&self, phc: &str) -> Result<bool> {
        let parsed = PasswordHash::new(phc).map_err(map_phc_err)?;
        // If the stored hash isn't argon2 at all, we should re-hash.
        if parsed.algorithm.as_str() != Algorithm::Argon2id.ident().as_str() {
            return Ok(true);
        }
        let stored = Params::try_from(&parsed).map_err(map_phc_err)?;
        Ok(stored.m_cost() != self.params.m_cost()
            || stored.t_cost() != self.params.t_cost()
            || stored.p_cost() != self.params.p_cost())
    }
}

fn map_phc_err(e: password_hash::Error) -> Error {
    Error::Backend(anyhow::anyhow!("argon2: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify_round_trip() {
        let hasher = PasswordHasher::default();
        let phc = hasher.hash("hunter2").unwrap();
        assert!(phc.starts_with("$argon2id$"));
        assert!(hasher.verify("hunter2", &phc).unwrap());
    }

    #[test]
    fn wrong_password_returns_ok_false() {
        let hasher = PasswordHasher::default();
        let phc = hasher.hash("correct").unwrap();
        assert!(!hasher.verify("incorrect", &phc).unwrap());
    }

    #[test]
    fn malformed_hash_returns_err() {
        let hasher = PasswordHasher::default();
        let result = hasher.verify("anything", "not-a-phc-string");
        assert!(matches!(result, Err(Error::Backend(_))));
    }

    #[test]
    fn salt_is_per_call_so_two_hashes_of_same_input_differ() {
        let hasher = PasswordHasher::default();
        let a = hasher.hash("same").unwrap();
        let b = hasher.hash("same").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn needs_rehash_false_for_same_params() {
        let hasher = PasswordHasher::default();
        let phc = hasher.hash("pw").unwrap();
        assert!(!hasher.needs_rehash(&phc).unwrap());
    }

    #[test]
    fn needs_rehash_true_when_params_drift() {
        let weak = PasswordHasher::with_params(Params::new(8, 1, 1, None).unwrap());
        let phc = weak.hash("pw").unwrap();
        let strong = PasswordHasher::default();
        assert!(strong.needs_rehash(&phc).unwrap());
    }
}
