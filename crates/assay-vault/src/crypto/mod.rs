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
pub mod kek_seal;
pub mod seal_state;
pub mod sealing;

#[cfg(any(feature = "backend-postgres", feature = "backend-sqlite"))]
pub mod kek_rotate;

#[cfg(any(feature = "backend-postgres", feature = "backend-sqlite"))]
pub mod kek_store;

pub use aead::{NONCE_LEN, decrypt, encrypt, random_dek, random_nonce};
pub use kek::KekHandle;
pub use kek_seal::{METHOD_SEALED_V1, UnsealMaterial, UnsealSource, seal_kek, unseal_kek};
pub use seal_state::{SealState, SealStatus};
pub use sealing::{KmsSeal, SealStore, SealingMethod};
