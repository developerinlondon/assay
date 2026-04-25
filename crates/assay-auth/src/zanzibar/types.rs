//! Plain-old-data types for the Zanzibar / ReBAC layer.
//!
//! Mirrors the Google Zanzibar paper terminology (Keto / SpiceDB users
//! will recognise the names):
//!
//! - **object** — a resource being protected, identified as
//!   `<type>:<id>` (e.g. `document:foo`, `circle:immediate`).
//! - **subject** — who's being checked. Either a *direct* user
//!   (`user:alice`, `subject_rel = None`) or a *userset* — every member
//!   of some other relation (`family:foo#member`, where
//!   `subject_rel = Some("member")`).
//! - **tuple** — the atomic permission fact:
//!   `object#relation @ subject`. The persistence layer stores
//!   millions of these; the recursive-CTE walks them transitively.
//! - **namespace schema** — the authoritative description of which
//!   relations + permissions a given `object_type` supports, parsed
//!   from a SpiceDB-compatible DSL by [`super::schema`].
//!
//! All identifiers are owned `String`s — we don't intern. Tuples are
//! short-lived in memory; the SQL layer is where dense storage lives.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// `<type>:<id>` reference to a protected resource (the *object* side
/// of a relation tuple). Field name is `object_type`/`object_id` to
/// match the column names in `auth.zanzibar_tuples` 1:1 — keeps SQL
/// hand-rolled queries readable.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObjectRef {
    pub object_type: String,
    pub object_id: String,
}

impl ObjectRef {
    /// Convenience constructor — `ObjectRef::new("document", "foo")`.
    pub fn new(ty: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            object_type: ty.into(),
            object_id: id.into(),
        }
    }

    /// Parse `"<type>:<id>"`. Returns `None` if no `:` separator is
    /// present or either side is empty — callers wrap this in a typed
    /// error appropriate to their context (HTTP 400, parser line/col,
    /// etc.).
    pub fn parse(s: &str) -> Option<Self> {
        let (ty, id) = s.split_once(':')?;
        if ty.is_empty() || id.is_empty() {
            return None;
        }
        Some(Self::new(ty, id))
    }

    /// `<type>:<id>` rendering. Round-trips with [`Self::parse`].
    pub fn render(&self) -> String {
        format!("{}:{}", self.object_type, self.object_id)
    }
}

/// `<type>:<id>[#<relation>]` reference. A subject is either:
///
/// - a **direct** user (`subject_rel = None`) — terminal, e.g.
///   `user:alice`, that's the leaf the recursive CTE walks toward.
/// - a **userset** (`subject_rel = Some("member")`) — every member of
///   `<type>:<id>`'s `relation`, e.g. `family:smith#member`. The walk
///   follows these one hop at a time.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubjectRef {
    pub subject_type: String,
    pub subject_id: String,
    pub subject_rel: Option<String>,
}

impl SubjectRef {
    pub fn direct(ty: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            subject_type: ty.into(),
            subject_id: id.into(),
            subject_rel: None,
        }
    }

    pub fn userset(
        ty: impl Into<String>,
        id: impl Into<String>,
        relation: impl Into<String>,
    ) -> Self {
        Self {
            subject_type: ty.into(),
            subject_id: id.into(),
            subject_rel: Some(relation.into()),
        }
    }

    /// Parse `"<type>:<id>"` (direct) or `"<type>:<id>#<relation>"`
    /// (userset). Returns `None` if the structural shape is invalid.
    pub fn parse(s: &str) -> Option<Self> {
        let (head, rel) = match s.split_once('#') {
            Some((h, r)) if !r.is_empty() => (h, Some(r.to_string())),
            Some(_) => return None,
            None => (s, None),
        };
        let (ty, id) = head.split_once(':')?;
        if ty.is_empty() || id.is_empty() {
            return None;
        }
        Some(Self {
            subject_type: ty.to_string(),
            subject_id: id.to_string(),
            subject_rel: rel,
        })
    }

    /// Round-trip rendering with [`Self::parse`].
    pub fn render(&self) -> String {
        match &self.subject_rel {
            Some(r) => format!("{}:{}#{}", self.subject_type, self.subject_id, r),
            None => format!("{}:{}", self.subject_type, self.subject_id),
        }
    }
}

/// One row of `auth.zanzibar_tuples`. Field names mirror the columns
/// 1:1 so hand-rolled SQL stays readable.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tuple {
    pub object_type: String,
    pub object_id: String,
    pub relation: String,
    pub subject_type: String,
    pub subject_id: String,
    pub subject_rel: Option<String>,
}

impl Tuple {
    /// Direct grant — `user:alice` is `viewer` of `document:foo`.
    pub fn direct(
        object: impl Into<ObjectRef>,
        relation: impl Into<String>,
        subject: impl Into<SubjectRef>,
    ) -> Self {
        let o: ObjectRef = object.into();
        let s: SubjectRef = subject.into();
        Self {
            object_type: o.object_type,
            object_id: o.object_id,
            relation: relation.into(),
            subject_type: s.subject_type,
            subject_id: s.subject_id,
            subject_rel: s.subject_rel,
        }
    }

    pub fn object(&self) -> ObjectRef {
        ObjectRef::new(self.object_type.clone(), self.object_id.clone())
    }

    pub fn subject(&self) -> SubjectRef {
        SubjectRef {
            subject_type: self.subject_type.clone(),
            subject_id: self.subject_id.clone(),
            subject_rel: self.subject_rel.clone(),
        }
    }
}

/// Read-consistency mode for `check`-style queries. Closely matches
/// the Zanzibar paper terminology and the SpiceDB API surface.
///
/// - [`Consistency::Minimum`] — read at any committed snapshot. Fastest,
///   no staleness bound. Default for non-critical UI checks.
/// - [`Consistency::AtLeastAsFresh`] — read at a snapshot at least as
///   recent as the provided zookie. Used right after a write to read
///   one's own writes.
/// - [`Consistency::Exact`] — read at exactly this snapshot. Used for
///   cache-friendly batched checks where every check should see the
///   same world.
///
/// In v0.2.0 zookies are opaque transaction-id strings; the Postgres
/// backend serialises `pg_current_wal_lsn()` and the SQLite backend
/// uses a monotonic counter. The check implementation in phase 6 is
/// `Consistency::Minimum` only (the other modes pass through to the
/// same code path) — full snapshot enforcement lands in phase 9.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Consistency {
    Minimum,
    AtLeastAsFresh(String),
    Exact(String),
}

impl Default for Consistency {
    fn default() -> Self {
        Consistency::Minimum
    }
}

/// Result of a `check` call. `Allowed` carries the (best-effort) tuple
/// path that resolved the permission so callers can show "why?" in a
/// debug UI; the path may be empty if the storage layer chose to skip
/// it for performance.
///
/// `DepthExceeded` and `CycleDetected` are *not* errors in the
/// `Result` sense — they're a deliberate denial signal. A buggy schema
/// shouldn't crash the request; it should deny the access and let the
/// operator inspect the response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckResult {
    Allowed { resolved_via: Vec<Tuple> },
    Denied,
    DepthExceeded,
    CycleDetected,
}

impl CheckResult {
    /// `true` iff [`CheckResult::Allowed`] — convenient for `if check.is_allowed()`.
    pub fn is_allowed(&self) -> bool {
        matches!(self, CheckResult::Allowed { .. })
    }
}

/// Tree returned by [`super::ZanzibarStore::expand`]. Models the
/// Zanzibar paper's "userset rewrite tree":
///
/// - [`UsersetTree::Leaf`] — terminal, a concrete user (or any
///   no-relation subject).
/// - [`UsersetTree::Node`] — an interior node showing how the
///   permission was decomposed (union/intersect/exclude) plus the
///   resolved children.
///
/// Mostly diagnostic — used by admin tooling and tests. The hot
/// `check` path doesn't materialise a full tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UsersetTree {
    Leaf {
        subject: SubjectRef,
    },
    Node {
        op: TreeOp,
        children: Vec<UsersetTree>,
    },
}

/// How a non-leaf [`UsersetTree`] node combines its children.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeOp {
    Union,
    Intersect,
    Exclude,
    /// `viewer` resolved by following the named relation tuples
    /// directly — the most common shape, e.g. `permission view = viewer`.
    Direct,
    /// Userset rewrite via `relation->permission` arrow.
    TuplesetArrow,
}

/// Persisted namespace definition — written by `define_namespace`,
/// read back by every `check` to resolve a permission name to its
/// underlying relation set.
///
/// Kept simple on purpose: the parsed [`super::schema`] AST round-
/// trips through `serde_json` into `auth.zanzibar_namespaces.schema_json`,
/// so adding a new permission shape later only needs a parser change,
/// not a storage migration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceSchema {
    pub name: String,
    /// Ordered map keyed by relation/permission name. `BTreeMap` keeps
    /// JSON serialisation stable across runs (matters for diff-friendly
    /// `auth.zanzibar_namespaces.schema_json` history).
    pub definitions: BTreeMap<String, RelationDef>,
}

impl NamespaceSchema {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            definitions: BTreeMap::new(),
        }
    }

    pub fn with_relation(mut self, name: impl Into<String>, def: RelationDef) -> Self {
        self.definitions.insert(name.into(), def);
        self
    }
}

/// A single line in a SpiceDB schema — `relation owner: user`,
/// `permission view = owner + viewer`, etc. Holds either the parsed
/// type list (for `relation` lines) or the algebraic expression (for
/// `permission` lines).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationDef {
    pub name: String,
    pub kind: RelationKind,
}

impl RelationDef {
    pub fn relation(name: impl Into<String>, types: Vec<TypeRef>) -> Self {
        Self {
            name: name.into(),
            kind: RelationKind::Direct(types),
        }
    }

    pub fn permission(name: impl Into<String>, expr: PermissionExpr) -> Self {
        Self {
            name: name.into(),
            kind: RelationKind::Permission(Box::new(expr)),
        }
    }
}

/// Categorises a definition line.
///
/// - [`RelationKind::Direct`] — `relation NAME: TYPE_LIST`. Only direct
///   tuples count (no rewrite expansion).
/// - [`RelationKind::Permission`] — `permission NAME = EXPR`. The
///   expression is composed of unions / intersects / exclusions /
///   tupleset arrows over relation names defined elsewhere in the
///   namespace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum RelationKind {
    Direct(Vec<TypeRef>),
    Permission(Box<PermissionExpr>),
}

/// A type reference on the right-hand side of a `relation` line.
/// `user` is `TypeRef::direct("user")`; `family#member` is
/// `TypeRef::userset("family", "member")`; `user:*` is the wildcard form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeRef {
    pub object_type: String,
    /// Userset reference — `family#member`. `None` = a direct subject.
    pub relation: Option<String>,
    /// Wildcard subject id — `user:*`. When `true` the parser saw
    /// `user:*` (any user is allowed) instead of just `user`. Treated
    /// as a permission shape rather than a sentinel value at the SQL
    /// layer.
    pub wildcard: bool,
}

impl TypeRef {
    pub fn direct(ty: impl Into<String>) -> Self {
        Self {
            object_type: ty.into(),
            relation: None,
            wildcard: false,
        }
    }

    pub fn userset(ty: impl Into<String>, relation: impl Into<String>) -> Self {
        Self {
            object_type: ty.into(),
            relation: Some(relation.into()),
            wildcard: false,
        }
    }

    pub fn wildcard(ty: impl Into<String>) -> Self {
        Self {
            object_type: ty.into(),
            relation: None,
            wildcard: true,
        }
    }
}

/// Algebraic permission expression — the right-hand side of a
/// `permission NAME = EXPR` line.
///
/// Composes via:
///
/// - [`PermissionExpr::Direct`] — name of a relation/permission to
///   resolve directly. The base case.
/// - [`PermissionExpr::Union`] / `Intersect` / `Exclude` — set ops
///   over two child expressions. Parsed as left-associative.
/// - [`PermissionExpr::TuplesetArrow`] — `relation->permission` — for
///   each tuple `(object, relation, intermediate_subject)`, recurse
///   into `intermediate_subject` checking `permission`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PermissionExpr {
    Direct {
        relation: String,
    },
    Union {
        left: Box<PermissionExpr>,
        right: Box<PermissionExpr>,
    },
    Intersect {
        left: Box<PermissionExpr>,
        right: Box<PermissionExpr>,
    },
    Exclude {
        left: Box<PermissionExpr>,
        right: Box<PermissionExpr>,
    },
    TuplesetArrow {
        tupleset: String,
        permission: String,
    },
}

impl PermissionExpr {
    pub fn direct(relation: impl Into<String>) -> Self {
        Self::Direct {
            relation: relation.into(),
        }
    }

    pub fn union(l: PermissionExpr, r: PermissionExpr) -> Self {
        Self::Union {
            left: Box::new(l),
            right: Box::new(r),
        }
    }

    pub fn intersect(l: PermissionExpr, r: PermissionExpr) -> Self {
        Self::Intersect {
            left: Box::new(l),
            right: Box::new(r),
        }
    }

    pub fn exclude(l: PermissionExpr, r: PermissionExpr) -> Self {
        Self::Exclude {
            left: Box::new(l),
            right: Box::new(r),
        }
    }

    pub fn arrow(tupleset: impl Into<String>, permission: impl Into<String>) -> Self {
        Self::TuplesetArrow {
            tupleset: tupleset.into(),
            permission: permission.into(),
        }
    }
}

/// Maximum recursion depth for `check` / `expand` walks. Matches plan
/// 11's choice and the SpiceDB default. A real-world Zanzibar
/// deployment rarely exceeds depth ~10; 50 leaves headroom for
/// pathological-but-legitimate schemas (deeply nested groups).
pub const MAX_DEPTH: u32 = 50;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_round_trips() {
        let o = ObjectRef::new("document", "foo");
        assert_eq!(o.render(), "document:foo");
        assert_eq!(ObjectRef::parse("document:foo"), Some(o));
        assert_eq!(ObjectRef::parse(""), None);
        assert_eq!(ObjectRef::parse("nope"), None);
        assert_eq!(ObjectRef::parse("a:"), None);
    }

    #[test]
    fn subject_round_trips() {
        let direct = SubjectRef::direct("user", "alice");
        assert_eq!(direct.render(), "user:alice");
        assert_eq!(SubjectRef::parse("user:alice"), Some(direct));

        let userset = SubjectRef::userset("family", "ahmed", "member");
        assert_eq!(userset.render(), "family:ahmed#member");
        assert_eq!(SubjectRef::parse("family:ahmed#member"), Some(userset));

        // Reject empty parts.
        assert_eq!(SubjectRef::parse(""), None);
        assert_eq!(SubjectRef::parse("user:#member"), None);
        assert_eq!(SubjectRef::parse("family:ahmed#"), None);
    }

    #[test]
    fn check_result_is_allowed() {
        assert!(CheckResult::Allowed { resolved_via: vec![] }.is_allowed());
        assert!(!CheckResult::Denied.is_allowed());
        assert!(!CheckResult::DepthExceeded.is_allowed());
        assert!(!CheckResult::CycleDetected.is_allowed());
    }
}
