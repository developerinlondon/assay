//! Backend-agnostic permission-expression evaluator — the recursive
//! Zanzibar `check` algebra shared by [`super::postgres`] and
//! [`super::sqlite`].
//!
//! The recursive-CTE walk each backend ships answers exactly one
//! question: *does `subject` reach `object` along any of a flat set of
//! relations, following userset hops?* That is sufficient for a
//! permission that flattens to a pure union of relations
//! (`view = owner + viewer`), but it cannot express the other three
//! [`PermissionExpr`] shapes:
//!
//! - **`TuplesetArrow`** (`parent->up`) — follow the `parent` tuples to
//!   another *object*, then re-evaluate `up` over there. The target
//!   object may live in a different namespace, so the permission has to
//!   be re-resolved against that object's schema each hop. A single CTE
//!   that only walks userset subjects (`subject_rel <> ''`) treats a
//!   direct object-reference tuple as terminal and never makes the hop.
//! - **`Intersect`** / **`Exclude`** — require evaluating both sides
//!   independently and composing the verdicts; "any of these relations
//!   matched" over-grants.
//!
//! This module owns that composition. Each backend supplies a small
//! set of async primitives via [`LeafCheck`]:
//!
//! 1. [`LeafCheck::check_relation_set`] — the single-CTE walk over a
//!    flat relation set (userset chains + the in-CTE depth/cycle bound
//!    live there). This is the leaf the algebra bottoms out at.
//! 2. [`LeafCheck::arrow_targets`] — list the *object* subjects of
//!    `(object, relation)` tuples (the left edge of an arrow hop).
//! 3. [`LeafCheck::schema_for`] — fetch a namespace schema so an arrow
//!    hop can re-resolve the target permission against the target
//!    object's definition.
//!
//! Recursion is bounded the same way the CTE bounds userset chains:
//! [`MAX_DEPTH`] frames on a `(object_type, object_id, name)` stack. A
//! revisit of that triple is a *data* cycle (an arrow graph that loops
//! back to an object/permission already being evaluated); it prunes the
//! branch to `Denied`, mirroring the leaf CTE's `path` guard for userset
//! cycles. [`CheckResult::CycleDetected`] is reserved for a malformed
//! *schema* — a permission expression that references itself — surfaced
//! by [`resolve`] returning `None`. Three-valued logic is fail-closed: a
//! branch that can't be resolved (depth/cycle) never grants; only a
//! definitive `Allowed` does.

use super::resolve::resolve;
use super::types::{
    CheckResult, MAX_DEPTH, NamespaceSchema, ObjectRef, PermissionExpr, RelationDef, RelationKind,
    SubjectRef,
};

use std::future::Future;
use std::pin::Pin;

/// Verdict of one evaluation node. A superset of the boolean
/// allow/deny: the two indeterminate variants carry *why* a branch
/// could not be resolved so the algebra can fail closed and the final
/// [`CheckResult`] can surface the reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allowed,
    Denied,
    DepthExceeded,
    CycleDetected,
}

impl Verdict {
    fn into_result(self) -> CheckResult {
        match self {
            Verdict::Allowed => CheckResult::Allowed {
                resolved_via: Vec::new(),
            },
            Verdict::Denied => CheckResult::Denied,
            Verdict::DepthExceeded => CheckResult::DepthExceeded,
            Verdict::CycleDetected => CheckResult::CycleDetected,
        }
    }
}

/// The storage primitives the evaluator composes: the single-relation
/// leaf check, the arrow-target listing, and the schema fetch an arrow
/// hop needs. Implemented once per backend over its own pool + SQL
/// dialect; the algebra above stays backend-agnostic.
pub trait LeafCheck: Sync {
    /// Single-relation-set leaf check: does `subject` reach `object`
    /// along any relation in `relations`, following userset hops? This
    /// is the existing recursive CTE, lifted to take the resolved
    /// relation set as input. Returns the CTE's own three-valued
    /// verdict (incl. its internal depth/cycle bound on userset chains).
    fn check_relation_set<'a>(
        &'a self,
        object: &'a ObjectRef,
        relations: &'a [String],
        subject: &'a SubjectRef,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Verdict>> + Send + 'a>>;

    /// List the *object* subjects of `(object, relation)` tuples — the
    /// left edge of a `relation->permission` arrow. Only object
    /// references (`subject_rel == ""`) participate in an arrow hop; a
    /// userset subject on the tupleset relation is skipped (it has no
    /// object to re-evaluate the target permission against).
    fn arrow_targets<'a>(
        &'a self,
        object: &'a ObjectRef,
        relation: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<ObjectRef>>> + Send + 'a>>;

    /// Fetch a namespace schema by type name. Used to re-resolve an
    /// arrow's target permission against the target object's definition.
    fn schema_for<'a>(
        &'a self,
        object_type: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Option<NamespaceSchema>>> + Send + 'a>>;
}

/// Evaluate `permission` on `object` for `subject`, resolving the full
/// [`PermissionExpr`] algebra (union / intersect / exclude / arrow)
/// against `backend`'s primitives.
///
/// `entry_schema` is `object`'s namespace (already fetched by the
/// caller's `check`). Arrow hops fetch the target object's schema on
/// demand via [`LeafCheck::schema_for`].
pub async fn evaluate<B: LeafCheck>(
    backend: &B,
    entry_schema: &NamespaceSchema,
    object: &ObjectRef,
    permission: &str,
    subject: &SubjectRef,
) -> anyhow::Result<CheckResult> {
    let mut frames: Vec<(String, String, String)> = Vec::new();
    let v = eval_name(
        backend,
        entry_schema,
        object,
        permission,
        subject,
        &mut frames,
    )
    .await?;
    Ok(v.into_result())
}

/// Evaluate a relation- or permission-*name* on `object`. This is where
/// the depth/cycle frame is pushed and where the pure-union fast path is
/// taken: a name whose definition flattens to a flat union of relations
/// (no arrow/intersect/exclude anywhere beneath it) is answered by a
/// single leaf CTE — identical shape and round-trip count to the
/// pre-arrow implementation.
fn eval_name<'a, B: LeafCheck>(
    backend: &'a B,
    schema: &'a NamespaceSchema,
    object: &'a ObjectRef,
    name: &'a str,
    subject: &'a SubjectRef,
    frames: &'a mut Vec<(String, String, String)>,
) -> Pin<Box<dyn Future<Output = anyhow::Result<Verdict>> + Send + 'a>> {
    Box::pin(async move {
        let frame = (
            object.object_type.clone(),
            object.object_id.clone(),
            name.to_string(),
        );
        // A data cycle: this `(object, permission)` is already on the
        // stack, reached again via an arrow hop. Prune the branch with a
        // plain `Denied` — the same thing the leaf CTE's `path` guard
        // does for a userset cycle (it stops expanding the repeated node
        // and contributes no rows). `CycleDetected` is reserved for a
        // malformed *schema* (a permission expression that references
        // itself), surfaced by `resolve` returning `None` below.
        if frames.contains(&frame) {
            return Ok(Verdict::Denied);
        }
        if frames.len() >= MAX_DEPTH as usize {
            return Ok(Verdict::DepthExceeded);
        }
        frames.push(frame);
        let out = eval_name_inner(backend, schema, object, name, subject, frames).await;
        frames.pop();
        out
    })
}

async fn eval_name_inner<B: LeafCheck>(
    backend: &B,
    schema: &NamespaceSchema,
    object: &ObjectRef,
    name: &str,
    subject: &SubjectRef,
    frames: &mut Vec<(String, String, String)>,
) -> anyhow::Result<Verdict> {
    match schema.definitions.get(name) {
        // A real relation, or an undefined name treated leniently as
        // one (matches `resolve`): the CTE walks its tuples directly.
        None
        | Some(RelationDef {
            kind: RelationKind::Direct(_),
            ..
        }) => {
            let relations = [name.to_string()];
            backend
                .check_relation_set(object, &relations, subject)
                .await
        }
        Some(RelationDef {
            kind: RelationKind::Permission(expr),
            ..
        }) => {
            // Fast path: a permission that flattens to a pure union of
            // relations (no arrow/intersect/exclude) is one CTE over the
            // whole seed set. `resolve` already computes that set and
            // flags whether algebra remains. `resolve` returning `None`
            // is a *schema* cycle (the permission expression references
            // itself, independent of any tuples) — surface it as
            // `CycleDetected` exactly as the pre-evaluator `check` did.
            let Some(resolved) = resolve(schema, name) else {
                return Ok(Verdict::CycleDetected);
            };
            if !resolved.needs_post_filter {
                if resolved.union_relations.is_empty() {
                    return Ok(Verdict::Denied);
                }
                let relations: Vec<String> = resolved.union_relations.into_iter().collect();
                return backend
                    .check_relation_set(object, &relations, subject)
                    .await;
            }
            // Otherwise walk the expression structurally.
            eval_expr(backend, schema, object, expr, subject, frames).await
        }
    }
}

/// Recursively evaluate a [`PermissionExpr`] node against `(object,
/// subject)` within `schema` (the namespace `object` belongs to).
fn eval_expr<'a, B: LeafCheck>(
    backend: &'a B,
    schema: &'a NamespaceSchema,
    object: &'a ObjectRef,
    expr: &'a PermissionExpr,
    subject: &'a SubjectRef,
    frames: &'a mut Vec<(String, String, String)>,
) -> Pin<Box<dyn Future<Output = anyhow::Result<Verdict>> + Send + 'a>> {
    Box::pin(async move {
        match expr {
            PermissionExpr::Direct { relation } => {
                eval_name(backend, schema, object, relation, subject, frames).await
            }
            PermissionExpr::Union { left, right } => {
                let l = eval_expr(backend, schema, object, left, subject, frames).await?;
                if l == Verdict::Allowed {
                    return Ok(Verdict::Allowed);
                }
                let r = eval_expr(backend, schema, object, right, subject, frames).await?;
                Ok(combine_union(l, r))
            }
            PermissionExpr::Intersect { left, right } => {
                let l = eval_expr(backend, schema, object, left, subject, frames).await?;
                if l == Verdict::Denied {
                    return Ok(Verdict::Denied);
                }
                let r = eval_expr(backend, schema, object, right, subject, frames).await?;
                Ok(combine_intersect(l, r))
            }
            PermissionExpr::Exclude { left, right } => {
                let l = eval_expr(backend, schema, object, left, subject, frames).await?;
                if l == Verdict::Denied {
                    return Ok(Verdict::Denied);
                }
                let r = eval_expr(backend, schema, object, right, subject, frames).await?;
                Ok(combine_exclude(l, r))
            }
            PermissionExpr::TuplesetArrow {
                tupleset,
                permission,
            } => eval_arrow(backend, object, tupleset, permission, subject, frames).await,
        }
    })
}

/// Evaluate `tupleset->permission`: for every *object* subject `o` of
/// `(object, tupleset)`, re-resolve and evaluate `permission` on `o`
/// against `o`'s own namespace. Any [`Verdict::Allowed`] wins;
/// otherwise the strongest indeterminate signal is reported (fail
/// closed). A missing target namespace is a clean deny for that hop.
async fn eval_arrow<B: LeafCheck>(
    backend: &B,
    object: &ObjectRef,
    tupleset: &str,
    permission: &str,
    subject: &SubjectRef,
    frames: &mut Vec<(String, String, String)>,
) -> anyhow::Result<Verdict> {
    let targets = backend.arrow_targets(object, tupleset).await?;
    let mut acc = Verdict::Denied;
    for target in &targets {
        let Some(target_schema) = backend.schema_for(&target.object_type).await? else {
            continue;
        };
        let v = eval_name(backend, &target_schema, target, permission, subject, frames).await?;
        if v == Verdict::Allowed {
            return Ok(Verdict::Allowed);
        }
        acc = combine_union(acc, v);
    }
    Ok(acc)
}

/// `a OR b` — a definitive grant on either side wins regardless of the
/// other being indeterminate. Otherwise the strongest "couldn't
/// determine" signal survives so the caller can surface depth/cycle.
fn combine_union(a: Verdict, b: Verdict) -> Verdict {
    match (a, b) {
        (Verdict::Allowed, _) | (_, Verdict::Allowed) => Verdict::Allowed,
        (Verdict::DepthExceeded, _) | (_, Verdict::DepthExceeded) => Verdict::DepthExceeded,
        (Verdict::CycleDetected, _) | (_, Verdict::CycleDetected) => Verdict::CycleDetected,
        _ => Verdict::Denied,
    }
}

/// `a AND b` — a definitive deny on either side wins. Both `Allowed`
/// grants; otherwise an indeterminate side blocks the grant (fail
/// closed) and is propagated.
fn combine_intersect(a: Verdict, b: Verdict) -> Verdict {
    match (a, b) {
        (Verdict::Denied, _) | (_, Verdict::Denied) => Verdict::Denied,
        (Verdict::Allowed, Verdict::Allowed) => Verdict::Allowed,
        (Verdict::DepthExceeded, _) | (_, Verdict::DepthExceeded) => Verdict::DepthExceeded,
        (Verdict::CycleDetected, _) | (_, Verdict::CycleDetected) => Verdict::CycleDetected,
    }
}

/// `a AND NOT b` (`left - right`). `left` already known not to be
/// `Denied` (caller short-circuits). A definitive grant on `right`
/// excludes; a definitive deny on `right` with `left == Allowed`
/// admits. An indeterminate `right` cannot prove the subject is *not*
/// excluded, so it fails closed.
fn combine_exclude(left: Verdict, right: Verdict) -> Verdict {
    match (left, right) {
        (_, Verdict::Allowed) => Verdict::Denied,
        (Verdict::Allowed, Verdict::Denied) => Verdict::Allowed,
        // left indeterminate, or right indeterminate → cannot grant.
        (Verdict::DepthExceeded, _) | (_, Verdict::DepthExceeded) => Verdict::DepthExceeded,
        (Verdict::CycleDetected, _) | (_, Verdict::CycleDetected) => Verdict::CycleDetected,
        // left == Denied is handled by the caller's short-circuit; the
        // remaining (Denied, Denied) is a deny.
        _ => Verdict::Denied,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zanzibar::types::{RelationDef, TypeRef};
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Mutex;

    /// In-memory [`LeafCheck`] over a tuple set, mirroring the SQL
    /// backends without a database. The leaf check follows userset hops
    /// transitively (the CTE's job) with its own depth/cycle bound so
    /// the evaluator's composition can be unit-tested in isolation.
    struct FakeBackend {
        /// (object_type, object_id, relation) -> subjects.
        tuples: Vec<((String, String, String), SubjectRef)>,
        schemas: BTreeMap<String, NamespaceSchema>,
        /// Records each relation-set leaf check for fast-path assertions.
        leaf_calls: Mutex<Vec<Vec<String>>>,
    }

    impl FakeBackend {
        fn new(schemas: Vec<NamespaceSchema>) -> Self {
            Self {
                tuples: Vec::new(),
                schemas: schemas.into_iter().map(|s| (s.name.clone(), s)).collect(),
                leaf_calls: Mutex::new(Vec::new()),
            }
        }

        fn tuple(mut self, ot: &str, oid: &str, rel: &str, subject: SubjectRef) -> Self {
            self.tuples
                .push(((ot.into(), oid.into(), rel.into()), subject));
            self
        }

        /// Direct subjects of (object, relation).
        fn direct_subjects(&self, object: &ObjectRef, relation: &str) -> Vec<SubjectRef> {
            self.tuples
                .iter()
                .filter(|((ot, oid, rel), _)| {
                    ot == &object.object_type && oid == &object.object_id && rel == relation
                })
                .map(|(_, s)| s.clone())
                .collect()
        }
    }

    impl LeafCheck for FakeBackend {
        fn check_relation_set<'a>(
            &'a self,
            object: &'a ObjectRef,
            relations: &'a [String],
            subject: &'a SubjectRef,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<Verdict>> + Send + 'a>> {
            self.leaf_calls.lock().unwrap().push(relations.to_vec());
            Box::pin(async move {
                // BFS over userset hops, bounded like the real CTE.
                let mut seen: BTreeSet<(String, String, String)> = BTreeSet::new();
                let mut frontier: Vec<(ObjectRef, String)> = relations
                    .iter()
                    .map(|r| (object.clone(), r.clone()))
                    .collect();
                let mut depth = 0u32;
                while !frontier.is_empty() {
                    depth += 1;
                    if depth > MAX_DEPTH {
                        return Ok(Verdict::DepthExceeded);
                    }
                    let mut next = Vec::new();
                    for (obj, rel) in frontier.drain(..) {
                        for s in self.direct_subjects(&obj, &rel) {
                            if s.subject_rel.is_empty() {
                                if s.subject_type == subject.subject_type
                                    && s.subject_id == subject.subject_id
                                {
                                    return Ok(Verdict::Allowed);
                                }
                            } else {
                                let key = (
                                    s.subject_type.clone(),
                                    s.subject_id.clone(),
                                    s.subject_rel.clone(),
                                );
                                if seen.insert(key) {
                                    next.push((
                                        ObjectRef::new(s.subject_type, s.subject_id),
                                        s.subject_rel,
                                    ));
                                }
                            }
                        }
                    }
                    frontier = next;
                }
                Ok(Verdict::Denied)
            })
        }

        fn arrow_targets<'a>(
            &'a self,
            object: &'a ObjectRef,
            relation: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<ObjectRef>>> + Send + 'a>> {
            Box::pin(async move {
                Ok(self
                    .direct_subjects(object, relation)
                    .into_iter()
                    .filter(|s| s.subject_rel.is_empty())
                    .map(|s| ObjectRef::new(s.subject_type, s.subject_id))
                    .collect())
            })
        }

        fn schema_for<'a>(
            &'a self,
            object_type: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<Option<NamespaceSchema>>> + Send + 'a>>
        {
            Box::pin(async move { Ok(self.schemas.get(object_type).cloned()) })
        }
    }

    /// `node` namespace with `up = account + parent->up` — the oracle's
    /// hierarchy schema.
    fn node_schema() -> NamespaceSchema {
        NamespaceSchema::new("node")
            .with_relation(
                "account",
                RelationDef::relation("account", vec![TypeRef::direct("user")]),
            )
            .with_relation(
                "parent",
                RelationDef::relation("parent", vec![TypeRef::direct("node")]),
            )
            .with_relation(
                "up",
                RelationDef::permission(
                    "up",
                    PermissionExpr::union(
                        PermissionExpr::direct("account"),
                        PermissionExpr::arrow("parent", "up"),
                    ),
                ),
            )
    }

    async fn check(
        backend: &FakeBackend,
        schema: &NamespaceSchema,
        oid: &str,
        perm: &str,
        sid: &str,
    ) -> CheckResult {
        let object = ObjectRef::new(&schema.name, oid);
        let subject = SubjectRef::direct("user", sid);
        evaluate(backend, schema, &object, perm, &subject)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn arrow_resolves_hierarchy() {
        let schema = node_schema();
        let backend = FakeBackend::new(vec![schema.clone(), NamespaceSchema::new("user")])
            .tuple("node", "a", "account", SubjectRef::direct("user", "ua"))
            .tuple("node", "b", "account", SubjectRef::direct("user", "ub"))
            .tuple("node", "c", "account", SubjectRef::direct("user", "uc"))
            .tuple("node", "b", "parent", SubjectRef::direct("node", "a"))
            .tuple("node", "c", "parent", SubjectRef::direct("node", "b"));

        // grandparent account, via c->b->a
        assert!(check(&backend, &schema, "c", "up", "ua").await.is_allowed());
        // parent account (node with both account and parent)
        assert!(check(&backend, &schema, "c", "up", "ub").await.is_allowed());
        // own account
        assert!(check(&backend, &schema, "c", "up", "uc").await.is_allowed());
        // unrelated
        assert_eq!(
            check(&backend, &schema, "c", "up", "nobody").await,
            CheckResult::Denied
        );
    }

    #[tokio::test]
    async fn intersect_requires_both() {
        let schema = NamespaceSchema::new("doc")
            .with_relation(
                "a",
                RelationDef::relation("a", vec![TypeRef::direct("user")]),
            )
            .with_relation(
                "b",
                RelationDef::relation("b", vec![TypeRef::direct("user")]),
            )
            .with_relation(
                "p",
                RelationDef::permission(
                    "p",
                    PermissionExpr::intersect(
                        PermissionExpr::direct("a"),
                        PermissionExpr::direct("b"),
                    ),
                ),
            );
        let backend = FakeBackend::new(vec![schema.clone()])
            .tuple("doc", "x", "a", SubjectRef::direct("user", "alice"))
            .tuple("doc", "x", "b", SubjectRef::direct("user", "alice"))
            .tuple("doc", "x", "a", SubjectRef::direct("user", "bob"));

        assert!(
            check(&backend, &schema, "x", "p", "alice")
                .await
                .is_allowed()
        );
        // bob only has `a`, not `b`.
        assert_eq!(
            check(&backend, &schema, "x", "p", "bob").await,
            CheckResult::Denied
        );
    }

    #[tokio::test]
    async fn exclude_subtracts_right() {
        let schema = NamespaceSchema::new("doc")
            .with_relation(
                "viewer",
                RelationDef::relation("viewer", vec![TypeRef::direct("user")]),
            )
            .with_relation(
                "banned",
                RelationDef::relation("banned", vec![TypeRef::direct("user")]),
            )
            .with_relation(
                "view",
                RelationDef::permission(
                    "view",
                    PermissionExpr::exclude(
                        PermissionExpr::direct("viewer"),
                        PermissionExpr::direct("banned"),
                    ),
                ),
            );
        let backend = FakeBackend::new(vec![schema.clone()])
            .tuple("doc", "x", "viewer", SubjectRef::direct("user", "alice"))
            .tuple("doc", "x", "viewer", SubjectRef::direct("user", "mallory"))
            .tuple("doc", "x", "banned", SubjectRef::direct("user", "mallory"));

        assert!(
            check(&backend, &schema, "x", "view", "alice")
                .await
                .is_allowed()
        );
        // mallory is a viewer but banned.
        assert_eq!(
            check(&backend, &schema, "x", "view", "mallory").await,
            CheckResult::Denied
        );
    }

    #[tokio::test]
    async fn pure_union_uses_single_leaf_check() {
        let schema = NamespaceSchema::new("doc")
            .with_relation(
                "owner",
                RelationDef::relation("owner", vec![TypeRef::direct("user")]),
            )
            .with_relation(
                "viewer",
                RelationDef::relation("viewer", vec![TypeRef::direct("user")]),
            )
            .with_relation(
                "view",
                RelationDef::permission(
                    "view",
                    PermissionExpr::union(
                        PermissionExpr::direct("owner"),
                        PermissionExpr::direct("viewer"),
                    ),
                ),
            );
        let backend = FakeBackend::new(vec![schema.clone()]).tuple(
            "doc",
            "x",
            "owner",
            SubjectRef::direct("user", "alice"),
        );

        assert!(
            check(&backend, &schema, "x", "view", "alice")
                .await
                .is_allowed()
        );
        // The fast path issues exactly one leaf check carrying the whole
        // union {owner, viewer} — not one per branch.
        let calls = backend.leaf_calls.lock().unwrap();
        assert_eq!(
            calls.len(),
            1,
            "fast path should be a single leaf check: {calls:?}"
        );
        assert_eq!(
            calls[0].iter().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from(["owner".to_string(), "viewer".to_string()])
        );
    }

    #[tokio::test]
    async fn arrow_cycle_is_safe() {
        // A *data* cycle in the object graph: a.parent=b, b.parent=a,
        // with no account anywhere. The arrow recursion revisits the
        // (node:a, up) frame; that branch is pruned to `Denied` (the
        // same outcome the leaf CTE gives a userset cycle), so the walk
        // terminates and denies rather than looping. `CycleDetected` is
        // reserved for a malformed *schema* (see `schema_cycle_*`).
        let schema = node_schema();
        let backend = FakeBackend::new(vec![schema.clone(), NamespaceSchema::new("user")])
            .tuple("node", "a", "parent", SubjectRef::direct("node", "b"))
            .tuple("node", "b", "parent", SubjectRef::direct("node", "a"));
        assert_eq!(
            check(&backend, &schema, "a", "up", "ghost").await,
            CheckResult::Denied
        );
    }

    #[tokio::test]
    async fn arrow_cycle_still_resolves_real_account() {
        // The same cyclic parent graph, but `a` carries an account. The
        // cycle must not mask a legitimate grant reachable before the
        // branch is pruned.
        let schema = node_schema();
        let backend = FakeBackend::new(vec![schema.clone(), NamespaceSchema::new("user")])
            .tuple("node", "a", "parent", SubjectRef::direct("node", "b"))
            .tuple("node", "b", "parent", SubjectRef::direct("node", "a"))
            .tuple("node", "a", "account", SubjectRef::direct("user", "ua"));
        assert!(check(&backend, &schema, "b", "up", "ua").await.is_allowed());
    }

    #[tokio::test]
    async fn schema_cycle_reports_cycle_detected() {
        // A malformed schema: `p = q`, `q = p` — the permission
        // expression references itself with no tuples involved. This is
        // a distinct failure from a data cycle and surfaces as
        // `CycleDetected` (preserving the pre-evaluator `check` signal).
        let schema = NamespaceSchema::new("doc")
            .with_relation(
                "p",
                RelationDef::permission("p", PermissionExpr::direct("q")),
            )
            .with_relation(
                "q",
                RelationDef::permission("q", PermissionExpr::direct("p")),
            );
        let backend = FakeBackend::new(vec![schema.clone()]);
        assert_eq!(
            check(&backend, &schema, "x", "p", "alice").await,
            CheckResult::CycleDetected
        );
    }
}
