//! Permission-expression resolution helpers — backend-agnostic logic
//! shared by [`super::postgres`] and [`super::sqlite`].
//!
//! The recursive-CTE walk needs the *set of relation names* that
//! resolve to a permission so the seed `WHERE relation = ANY($3)` can
//! be evaluated by the database in one round-trip. This module crawls
//! the parsed [`PermissionExpr`] tree once at check time and yields:
//!
//! - **`UnionSet`** — every relation name visited under [`PermissionExpr::Union`]
//!   or [`PermissionExpr::Direct`]. Forms the seed set the walk
//!   expands from.
//! - **`Plan`** — the full algebraic structure annotated with which
//!   relation names belong to each leaf, used by the higher-level
//!   `check` after the walk to apply intersect/exclude composition.
//!   Today the storage layer only needs the union set; the plan is
//!   exposed so the upcoming intersect/exclude pass (also phase 6) has
//!   a stable input.
//!
//! Single-pass, allocation-light — the resolver is on the request hot
//! path so we keep it tight.

use std::collections::BTreeSet;

use super::types::{NamespaceSchema, PermissionExpr, RelationKind};

/// Resolved permission — for v0.2.0 the storage layer needs only the
/// flat union of "candidate relation names to seed the recursive walk
/// with". Intersect / exclude / arrow handling is layered on top of
/// this in `check`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// Relation names whose tuples participate in this permission.
    /// Includes both directly-referenced relations and those reached
    /// via union. For a pure union schema (`view = owner + viewer +
    /// editor`) this is the answer in one pass.
    pub union_relations: BTreeSet<String>,
    /// Whether the permission is composed of *more than* unions /
    /// directs — i.e. the storage walk's "any of these relations
    /// matches" answer is necessary but not sufficient. Phase 6 still
    /// returns `Allowed` for these (per plan: union/intersect/exclude
    /// scaffolded; full intersect/exclude semantics layered later).
    pub needs_post_filter: bool,
}

impl Resolved {
    fn new() -> Self {
        Self {
            union_relations: BTreeSet::new(),
            needs_post_filter: false,
        }
    }
}

/// Resolve `permission_or_relation` against `schema` to the set of
/// relation names the storage walk should seed from.
///
/// If `permission_or_relation` names a `relation` (not a `permission`),
/// the result is just `{name}` — relations are their own seed.
///
/// Cycles in the permission expression (a buggy schema referring to
/// itself) are caught and surfaced as `None`; callers map that to a
/// "deny" outcome rather than an error so a bad schema doesn't 500.
pub fn resolve(schema: &NamespaceSchema, permission_or_relation: &str) -> Option<Resolved> {
    let mut out = Resolved::new();
    let mut seen = BTreeSet::new();
    walk_name(schema, permission_or_relation, &mut out, &mut seen).then_some(out)
}

/// Recursive helper. `seen` is the visited-set used for cycle
/// detection — if `name` is already on the stack we abandon and let
/// the caller reject the schema.
fn walk_name(
    schema: &NamespaceSchema,
    name: &str,
    out: &mut Resolved,
    seen: &mut BTreeSet<String>,
) -> bool {
    if !seen.insert(name.to_string()) {
        return false;
    }
    let Some(def) = schema.definitions.get(name) else {
        // An undefined name — treat as a direct relation (matches
        // SpiceDB's lenient resolution; the tuple table just won't
        // have rows for it and the walk denies cleanly).
        out.union_relations.insert(name.to_string());
        return true;
    };
    match &def.kind {
        RelationKind::Direct(_) => {
            out.union_relations.insert(name.to_string());
            true
        }
        RelationKind::Permission(expr) => walk_expr(schema, expr, out, seen),
    }
}

fn walk_expr(
    schema: &NamespaceSchema,
    expr: &PermissionExpr,
    out: &mut Resolved,
    seen: &mut BTreeSet<String>,
) -> bool {
    match expr {
        PermissionExpr::Direct { relation } => walk_name(schema, relation, out, seen),
        PermissionExpr::Union { left, right } => {
            walk_expr(schema, left, out, seen) && walk_expr(schema, right, out, seen)
        }
        PermissionExpr::Intersect { left, right } => {
            out.needs_post_filter = true;
            walk_expr(schema, left, out, seen) && walk_expr(schema, right, out, seen)
        }
        PermissionExpr::Exclude { left, right } => {
            out.needs_post_filter = true;
            walk_expr(schema, left, out, seen) && walk_expr(schema, right, out, seen)
        }
        PermissionExpr::TuplesetArrow {
            tupleset,
            permission: _,
        } => {
            // Arrow is essentially "follow `tupleset` to the
            // intermediate object, then check `permission` there".
            // The walk is recursive in the storage layer (the userset
            // hop), so seeding the tupleset relation is sufficient at
            // this stage. We mark `needs_post_filter` so callers know
            // the in-DB walk's "any match" answer might over-grant
            // until the full arrow pass lands.
            out.needs_post_filter = true;
            out.union_relations.insert(tupleset.to_string());
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zanzibar::types::{NamespaceSchema, PermissionExpr, RelationDef, TypeRef};

    fn doc_schema() -> NamespaceSchema {
        NamespaceSchema::new("document")
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
            )
            .with_relation(
                "edit",
                RelationDef::permission("edit", PermissionExpr::direct("owner")),
            )
    }

    #[test]
    fn resolves_union_permission() {
        let r = resolve(&doc_schema(), "view").expect("resolve");
        assert_eq!(
            r.union_relations,
            BTreeSet::from(["owner".into(), "viewer".into()])
        );
        assert!(!r.needs_post_filter);
    }

    #[test]
    fn resolves_direct_permission() {
        let r = resolve(&doc_schema(), "edit").expect("resolve");
        assert_eq!(r.union_relations, BTreeSet::from(["owner".into()]));
    }

    #[test]
    fn resolves_relation_as_self() {
        let r = resolve(&doc_schema(), "owner").expect("resolve");
        assert_eq!(r.union_relations, BTreeSet::from(["owner".into()]));
    }

    #[test]
    fn cycle_detected() {
        let mut s = NamespaceSchema::new("x");
        s.definitions.insert(
            "a".into(),
            RelationDef::permission("a", PermissionExpr::direct("b")),
        );
        s.definitions.insert(
            "b".into(),
            RelationDef::permission("b", PermissionExpr::direct("a")),
        );
        assert!(resolve(&s, "a").is_none());
    }

    #[test]
    fn unknown_name_treated_as_direct() {
        let r = resolve(&doc_schema(), "ghost").expect("resolve");
        assert_eq!(r.union_relations, BTreeSet::from(["ghost".into()]));
    }

    #[test]
    fn intersect_marks_post_filter() {
        let mut s = NamespaceSchema::new("x");
        s.definitions.insert(
            "a".into(),
            RelationDef::relation("a", vec![TypeRef::direct("user")]),
        );
        s.definitions.insert(
            "b".into(),
            RelationDef::relation("b", vec![TypeRef::direct("user")]),
        );
        s.definitions.insert(
            "p".into(),
            RelationDef::permission(
                "p",
                PermissionExpr::intersect(PermissionExpr::direct("a"), PermissionExpr::direct("b")),
            ),
        );
        let r = resolve(&s, "p").expect("resolve");
        assert!(r.needs_post_filter);
        assert!(r.union_relations.contains("a"));
        assert!(r.union_relations.contains("b"));
    }
}
