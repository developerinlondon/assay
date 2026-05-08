//! Login-CSRF binding token — pin the in-flight upstream-OIDC `state`
//! row to the originating browser session.
//!
//! Threat: an attacker logs in to the upstream as themselves, intercepts
//! their own callback URL pre-redirect, and ships
//! `https://victim/oidc/upstream/<slug>/callback?code=X&state=Y` to the
//! victim. Without a per-session pin, the victim's browser arrives at a
//! valid in-flight state row, completes the upstream code-exchange, and
//! gets a session cookie minted for the *attacker's* upstream identity.
//!
//! Mitigation: when `start_upstream_login` writes the state row, it also
//! mints a 32-byte random token (`raw`), stores its SHA-256 hash on the
//! row (`binding_hash`), and sets `assay_oidc_binding=<raw>` as an
//! HttpOnly cookie scoped to `/oidc/upstream/`. The callback parses the
//! cookie, takes the row, and rejects the flow if
//! `verify(raw, &row.binding_hash) == false`.
//!
//! Constant-time hash compare via [`subtle::ConstantTimeEq`] — the hash
//! is short (32 bytes) so a non-CT compare would still be hard to
//! exploit, but the dep is already in the workspace and CT is the
//! correct primitive for "MAC-equivalent token".

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Produce a fresh `(raw_token, sha256_hex_hash)` pair. Caller stores
/// the hash on the state row, sends the raw via the cookie. Returning
/// the pair from a single helper prevents call sites from accidentally
/// swapping them.
pub fn generate() -> (String, String) {
    let mut bytes = [0u8; 32];
    use rand::RngCore;
    rand::rng().fill_bytes(&mut bytes);
    let raw = data_encoding::BASE64URL_NOPAD.encode(&bytes);
    let hash = sha256_hex(raw.as_bytes());
    (raw, hash)
}

/// Constant-time check that `raw` hashes to `hash`. `hash` is the lower
/// hex form of SHA-256(`raw`).
pub fn verify(raw: &str, hash: &str) -> bool {
    if raw.is_empty() || hash.is_empty() {
        return false;
    }
    let computed = sha256_hex(raw.as_bytes());
    computed.as_bytes().ct_eq(hash.as_bytes()).into()
}

fn sha256_hex(input: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(input);
    let digest = h.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_returns_distinct_pairs() {
        let (raw1, hash1) = generate();
        let (raw2, hash2) = generate();
        assert_ne!(raw1, raw2);
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn verify_matches_generated_pair() {
        let (raw, hash) = generate();
        assert!(verify(&raw, &hash));
    }

    #[test]
    fn verify_rejects_mismatch() {
        let (_raw, hash) = generate();
        assert!(!verify("not_the_token", &hash));
    }

    #[test]
    fn verify_rejects_empty() {
        assert!(!verify("", "deadbeef"));
        assert!(!verify("xx", ""));
    }

    #[test]
    fn sha256_hex_known_vector() {
        // `echo -n "abc" | sha256sum`
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
