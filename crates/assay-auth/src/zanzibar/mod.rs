//! Zanzibar / ReBAC layer — Keto / SpiceDB-equivalent authorization.
//!
//! Module shape:
//!
//! - [`types`] — POD types: [`Tuple`], [`ObjectRef`], [`SubjectRef`],
//!   [`NamespaceSchema`], [`PermissionExpr`], [`Consistency`],
//!   [`CheckResult`], [`UsersetTree`].
//! - [`schema`] — SpiceDB-compatible DSL parser.
//! - [`resolve`] — permission-expression → seed relation set
//!   computation, shared by both backend impls.
//! - [`store`] — the [`ZanzibarStore`] async trait.
//! - [`postgres`] / [`sqlite`] — recursive-CTE-backed implementations.
//!
//! Why a directory module: phase 6 is the largest single auth module
//! by line count. Splitting it keeps each file under ~500 LOC while
//! preserving the `assay_auth::zanzibar::ZanzibarStore` import shape
//! callers expect. Compile-time `auth-zanzibar` feature still gates
//! the entire tree.

pub mod eval;
#[cfg(feature = "backend-postgres")]
pub mod postgres;
pub mod resolve;
pub mod schema;
#[cfg(feature = "backend-sqlite")]
pub mod sqlite;
pub mod store;
pub mod types;

pub use schema::{ParseError, parse_schema};
pub use store::ZanzibarStore;
pub use types::{
    CheckResult, Consistency, MAX_DEPTH, NamespaceSchema, ObjectRef, PermissionExpr, RelationDef,
    RelationKind, SubjectRef, TreeOp, Tuple, TupleFilter, TypeRef, UsersetTree,
};

#[cfg(feature = "backend-postgres")]
pub use postgres::PostgresZanzibarStore;
#[cfg(feature = "backend-sqlite")]
pub use sqlite::SqliteZanzibarStore;
