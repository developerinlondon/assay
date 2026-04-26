//! Shared cloud-API primitives — the small set of HTTP-signing utilities
//! the AWS / GCP integrations need.
//!
//! Plan 17 §"Cost estimate" locks the AWS path to **minimal HTTPS +
//! signing** rather than the full SDK, since we make 2-3 calls per
//! provider and the SDK is wildly over-spec'd. This module ships the
//! ~150 LOC of sigv4 + JWT signing those calls share.
//!
//! ## Modules
//!
//! - [`sigv4`] — AWS Signature Version 4 helpers. Used by
//!   [`crate::sealing::kms::AwsKmsSeal`] (Phase 2) + the AWS dynamic-
//!   creds provider (Phase 5 §S3b) + the S3 audit sink (Phase 2 §S8).
//! - [`gcp_jwt`] — Google service-account JWT signing for the
//!   `https://oauth2.googleapis.com/token` exchange. Used by the GCP
//!   KMS auto-unseal + GCP dynamic-creds (service-account impersonation
//!   via `iamcredentials.googleapis.com`). Reuses the in-tree
//!   `jsonwebtoken` crate; no new code-signing primitives.

#[cfg(any(
    feature = "vault-sealing-kms",
    feature = "vault-dynamic-aws",
    feature = "vault-audit-forwarding",
))]
pub mod sigv4;

#[cfg(any(feature = "vault-sealing-kms", feature = "vault-dynamic-gcp"))]
pub mod gcp_jwt;
