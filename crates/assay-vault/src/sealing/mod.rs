//! KMS auto-unseal — AWS + GCP impls of the
//! [`crate::crypto::sealing::KmsSeal`] trait.
//!
//! Plan 17 §S7 ("Cloud KMS auto-unseal"). Operators set
//! `kek_metadata.sealing_method = 'kms-aws'` (or `'kms-gcp'`) and
//! point at a remote KMS key. On boot, the engine calls
//! `unwrap_kek` against `sealed_blob` and the unsealed bytes never
//! touch disk.

#[cfg(feature = "vault-sealing-kms")]
pub mod kms_aws;
#[cfg(feature = "vault-sealing-kms")]
pub mod kms_gcp;
