//! [`ZanzibarStore`] trait — the storage seam every backend implements.
//!
//! Object-safe via `async-trait` so [`crate::ctx::AuthCtx`] can hold
//! `Arc<dyn ZanzibarStore>`. The shape mirrors the SpiceDB / Keto API
//! surface so operators familiar with either can read assay's
//! implementation without context-switching:
//!
//! - `define_namespace` / `get_namespace` / `list_namespaces` — schema
//!   CRUD.
//! - `write_tuple` / `write_tuples` / `delete_tuple` — relation-tuple
//!   CRUD. Batch writes are atomic so a multi-tuple admin operation
//!   never half-applies.
//! - `check` — the hot path. Returns
//!   [`super::types::CheckResult::Allowed`] iff `subject` has
//!   `permission` on `object` per the namespace's resolved relation
//!   set.
//! - `expand` — diagnostic / admin tooling counterpart that returns
//!   the userset rewrite tree.
//! - `lookup_resources` / `lookup_subjects` — forward / reverse
//!   listings used by UI surfaces ("show every doc Alice can view",
//!   "show every viewer of doc X").
//!
//! Implementations live in [`super::postgres`] and [`super::sqlite`].

use super::types::{
    CheckResult, Consistency, NamespaceSchema, ObjectRef, SubjectRef, Tuple, TupleFilter,
    UsersetTree,
};

/// Async, object-safe Zanzibar storage trait. See module docs.
#[async_trait::async_trait]
pub trait ZanzibarStore: Send + Sync + 'static {
    /// Persist (or replace) a namespace's schema. Idempotent — caller
    /// may freely re-apply the same schema; a no-op insert/update is
    /// fine.
    async fn define_namespace(&self, schema: &NamespaceSchema) -> anyhow::Result<()>;

    /// Fetch a namespace's schema by name. `Ok(None)` if not yet
    /// defined — callers (typically `check`) treat that as a hard
    /// "deny" since the relation set can't be resolved.
    async fn get_namespace(&self, name: &str) -> anyhow::Result<Option<NamespaceSchema>>;

    /// List every namespace, ordered by name. Used by admin UI.
    async fn list_namespaces(&self) -> anyhow::Result<Vec<NamespaceSchema>>;

    /// Insert one tuple. Idempotent on the composite PK — re-writing
    /// the same tuple is a no-op (returns Ok(()) without erroring).
    async fn write_tuple(&self, tuple: &Tuple) -> anyhow::Result<()>;

    /// Atomic batch write. Either every tuple is persisted or none are.
    /// Used by the admin "import schema + seed tuples" workflows.
    async fn write_tuples(&self, tuples: &[Tuple]) -> anyhow::Result<()>;

    /// Delete one tuple by exact match. Returns `Ok(true)` iff a row
    /// was removed.
    async fn delete_tuple(&self, tuple: &Tuple) -> anyhow::Result<bool>;

    /// List tuples matching `filter`, ordered by
    /// (object_type, object_id, relation, subject_type, subject_id).
    /// Pagination via `limit` (default 100, max 1000) and `offset`.
    async fn list_tuples(&self, filter: &TupleFilter) -> anyhow::Result<Vec<Tuple>>;

    /// Permission check. Walks the tuple DAG from `resource` along
    /// every relation that resolves to `permission` per the namespace
    /// schema, looking for `subject`.
    ///
    /// `consistency` is captured today but treated uniformly as
    /// `Minimum` (any committed snapshot) — full snapshot enforcement
    /// is phase 9 work. The argument is preserved on the trait so
    /// that addition is a non-breaking change.
    async fn check(
        &self,
        resource: &ObjectRef,
        permission: &str,
        subject: &SubjectRef,
        consistency: Consistency,
    ) -> anyhow::Result<CheckResult>;

    /// Userset-rewrite expansion — returns the tree of subjects that
    /// satisfy `relation` on `resource`. Bounded by `depth_limit` to
    /// match the `check` walk's depth bound (caller passes
    /// [`super::types::MAX_DEPTH`] in production).
    async fn expand(
        &self,
        resource: &ObjectRef,
        relation: &str,
        depth_limit: u32,
    ) -> anyhow::Result<UsersetTree>;

    /// Forward index — find every `(resource_type, *)` where `subject`
    /// has `permission`. Used to populate UI lists like "every
    /// document Alice can view".
    async fn lookup_resources(
        &self,
        resource_type: &str,
        permission: &str,
        subject: &SubjectRef,
    ) -> anyhow::Result<Vec<ObjectRef>>;

    /// Reverse index — find every subject of type `subject_type` that
    /// has `permission` on `resource`. Used to populate UI lists like
    /// "every viewer of doc X".
    async fn lookup_subjects(
        &self,
        subject_type: &str,
        resource: &ObjectRef,
        permission: &str,
    ) -> anyhow::Result<Vec<SubjectRef>>;
}
