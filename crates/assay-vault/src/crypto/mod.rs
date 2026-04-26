//! Cryptographic primitives shared by KV, transit, sealing, and the
//! collection-key envelope flow.
//!
//! Plan 17 §"Crypto choices" (locked):
//! - KV at-rest:           AES-256-GCM-SIV, per-record DEK wrapped by KEK
//! - Transit:              AES-256-GCM-SIV, version in AAD
//! - Collection items E2E: AES-256-GCM-SIV, server cannot decrypt
//! - Master KEK:           256-bit symmetric, sealing-method-dependent at rest
//!
//! No custom protocols. No homebrew crypto. AES-GCM-SIV via RustCrypto's
//! `aes-gcm-siv` 0.11 — misuse-resistant AEAD that survives nonce reuse,
//! which matters for transit's deterministic per-version nonces.

pub mod aead;
pub mod kek;
pub mod sealing;

#[cfg(any(feature = "backend-postgres", feature = "backend-sqlite"))]
pub mod kek_store;

pub use aead::{decrypt, encrypt, random_dek, random_nonce, NONCE_LEN};
pub use kek::KekHandle;
pub use sealing::{KmsSeal, SealingMethod};
