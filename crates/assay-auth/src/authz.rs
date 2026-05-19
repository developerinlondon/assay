//! Pluggable authorization seam.
//!
//! Per the decoupled-modules architecture, the engine doesn't enforce
//! per-user policy at request time — that's the dashboard's job. But
//! deployments that want the engine to be the policy boundary (no
//! dashboard intermediation) can opt into an in-engine authz backend
//! via this trait.
//!
//! Implementations:
//!   * [`AssayZanzibarAuthz`] — wraps the in-process [`ZanzibarStore`]
//!     for deployments using assay-auth's own ReBAC engine.
//!   * [`NoneAuthz`] — always-allow. Use when the only gate at the
//!     engine boundary is bearer/JWT validity and policy lives entirely
//!     upstream.
//!
//! Future implementations may include HTTP clients to Keto, OPA, or
//! similar external authz services.

use std::sync::Arc;

/// Outcome of an [`Authz::check`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthzResult {
    Allowed,
    Denied,
    /// Authorization backend returned an error. Caller should fail
    /// closed (treat as Denied) but may want to log differently.
    Error(String),
}

/// Pluggable authorization backend.
///
/// `check` answers "does `subject_id` (a user identifier from a session
/// or JWT) have `relation` on `(namespace, object_id)`?" Object-typed
/// resources mirror the Zanzibar tuple shape — namespaces give types,
/// (namespace, object_id) identifies a specific instance, relation is
/// the permission/relation name.
#[async_trait::async_trait]
pub trait Authz: Send + Sync + 'static {
    async fn check(
        &self,
        namespace: &str,
        object_id: &str,
        relation: &str,
        subject_id: &str,
    ) -> AuthzResult;
}

/// Allow-all authz backend. Used when the engine is bearer-only and
/// policy decisions live entirely upstream (dashboard / BFF).
pub struct NoneAuthz;

#[async_trait::async_trait]
impl Authz for NoneAuthz {
    async fn check(&self, _ns: &str, _obj: &str, _rel: &str, _sub: &str) -> AuthzResult {
        AuthzResult::Allowed
    }
}

/// In-process Zanzibar adapter. Wraps a [`crate::zanzibar::ZanzibarStore`]
/// and translates `(namespace, object_id, relation, subject_id)` into the
/// store's `check` call with `subject_type = "user"`.
#[cfg(feature = "auth-zanzibar")]
pub struct AssayZanzibarAuthz {
    store: Arc<dyn crate::zanzibar::ZanzibarStore>,
}

#[cfg(feature = "auth-zanzibar")]
impl AssayZanzibarAuthz {
    pub fn new(store: Arc<dyn crate::zanzibar::ZanzibarStore>) -> Self {
        Self { store }
    }
}

#[cfg(feature = "auth-zanzibar")]
#[async_trait::async_trait]
impl Authz for AssayZanzibarAuthz {
    async fn check(
        &self,
        namespace: &str,
        object_id: &str,
        relation: &str,
        subject_id: &str,
    ) -> AuthzResult {
        use crate::zanzibar::{CheckResult, Consistency, ObjectRef, SubjectRef};
        let resource = ObjectRef {
            object_type: namespace.to_string(),
            object_id: object_id.to_string(),
        };
        let subject = SubjectRef {
            subject_type: "user".to_string(),
            subject_id: subject_id.to_string(),
            subject_rel: String::new(),
        };
        match self
            .store
            .check(&resource, relation, &subject, Consistency::Minimum)
            .await
        {
            Ok(CheckResult::Allowed { .. }) => AuthzResult::Allowed,
            Ok(_) => AuthzResult::Denied,
            Err(e) => AuthzResult::Error(format!("zanzibar check: {e}")),
        }
    }
}

/// Convenience alias for the trait object form used in engine config.
pub type DynAuthz = Arc<dyn Authz>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn none_authz_always_allows() {
        let a = NoneAuthz;
        assert_eq!(
            a.check("vault", "main", "access", "usr_x").await,
            AuthzResult::Allowed
        );
        assert_eq!(
            a.check("vault_path", "secret/prod", "reader", "usr_y")
                .await,
            AuthzResult::Allowed
        );
    }
}
