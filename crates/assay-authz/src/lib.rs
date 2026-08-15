//! An in-process authorization engine: policy statements, grants-at-scope
//! with typed bounds, ABAC conditions, deny-wins, asymmetric fail-closed.
//!
//! The golden fixtures under `conformance/cases` are the contract: this
//! engine decides every one of them identically to the reference library.
//! There is no storage layer — a host resolves its grants and hands them over.

pub mod action;
pub mod condition;
pub mod conditions;
pub mod describe;
pub mod engine;
pub mod evaluate;
pub mod model;
pub mod parse;
pub mod validate;

pub use action::{
    ActionCatalogue, ActionCatalogueEntry, ActionCatalogueError, ActionDerivation, ActionMatch,
};
pub use condition::{
    ConditionContext, ConditionKeySpec, ConditionKeyType, ConditionKeys, ConditionOperator,
    ContextValue, PolicyCondition,
};
pub use conditions::{ConditionsVerdict, eval_conditions};
pub use describe::{AuthzDescriptor, DESCRIPTOR_VERSION, describe};
pub use engine::{CheckDetail, CheckOptions, Engine, EngineConfig};
pub use evaluate::{
    Decision, EvaluateInput, Outcome, Query, Reason, applicable_grants, decide, evaluate,
};
pub use model::{
    Effect, GrantBounds, ResolvedGrant, Scope, ScopeEntry, Statement, Subject, SubjectEntry,
};
pub use parse::parse_rfc3339;
pub use validate::{Vocabulary, validate_conditions, validate_statements};
