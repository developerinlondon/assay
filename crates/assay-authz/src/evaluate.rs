//! The pure statement engine: given the applicable grants, deny-wins — a deny
//! granted at ANY scope in the resolved chain beats an allow from any other,
//! and nothing granted denies.

use serde_json::Value;

use crate::action::{ActionDerivation, ActionMatch, match_action};
use crate::condition::{ConditionContext, ConditionKeys};
use crate::conditions::{ConditionsVerdict, eval_conditions};
use crate::model::resource_matches;
use crate::model::{Effect, ResolvedGrant, ScopeEntry, Statement, SubjectEntry};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

/// Why the engine landed where it did. Not part of the decision contract —
/// the reference engine returns a bare verdict — but a check that denies is
/// only actionable if the caller can tell a deny from an absence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Allowed,
    ExplicitDeny,
    NoMatchingGrant,
    MalformedScopeChain,
    UndeclaredScopeKind,
    AdminBypass,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::ExplicitDeny => "explicit_deny",
            Self::NoMatchingGrant => "no_matching_grant",
            Self::MalformedScopeChain => "malformed_scope_chain",
            Self::UndeclaredScopeKind => "undeclared_scope_kind",
            Self::AdminBypass => "admin_bypass",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub decision: Decision,
    pub reason: Reason,
}

impl Outcome {
    pub fn allowed(&self) -> bool {
        self.decision == Decision::Allow
    }

    pub(crate) fn deny(reason: Reason) -> Self {
        Self {
            decision: Decision::Deny,
            reason,
        }
    }

    fn allow() -> Self {
        Self {
            decision: Decision::Allow,
            reason: Reason::Allowed,
        }
    }
}

/// One question, plus everything needed to answer it apart from the grants.
pub struct Query<'a> {
    pub action: &'a str,
    pub resource: &'a str,
    pub context: &'a ConditionContext,
    pub condition_keys: &'a ConditionKeys,
    pub action_derivation: Option<&'a ActionDerivation>,
}

pub struct EvaluateInput<'a> {
    pub grants: &'a [ResolvedGrant],
    pub subjects: &'a [SubjectEntry],
    pub scope_chain: &'a [ScopeEntry],
    pub query: Query<'a>,
    /// When declared, every chain entry's kind must be one of these. An empty
    /// declaration therefore admits no scope at all.
    pub scope_kinds: Option<&'a [String]>,
}

/// Validate inputs fail-closed, filter the applicable grants, decide
/// deny-wins.
pub fn evaluate(input: &EvaluateInput<'_>) -> Outcome {
    if let Some(refusal) = refuse_bad_chain(input.scope_chain, input.scope_kinds) {
        return refusal;
    }
    let grants = applicable_grants(input.grants, input.subjects, input.scope_chain);
    decide(&grants, &input.query)
}

/// A malformed or undeclared scope must never widen to "evaluate some other
/// chain instead" — it denies outright.
pub fn refuse_bad_chain(
    scope_chain: &[ScopeEntry],
    scope_kinds: Option<&[String]>,
) -> Option<Outcome> {
    for entry in scope_chain {
        let Some(scope) = entry.valid() else {
            return Some(Outcome::deny(Reason::MalformedScopeChain));
        };
        if let Some(kinds) = scope_kinds
            && !kinds.iter().any(|kind| kind == &scope.kind)
        {
            return Some(Outcome::deny(Reason::UndeclaredScopeKind));
        }
    }
    None
}

/// Deny-wins over an already-applicable grant set: an explicit deny beats any
/// allow; nothing matched denies.
pub fn decide(grants: &[&ResolvedGrant], query: &Query<'_>) -> Outcome {
    if has(grants, Effect::Deny, query) {
        return Outcome::deny(Reason::ExplicitDeny);
    }
    if has(grants, Effect::Allow, query) {
        Outcome::allow()
    } else {
        Outcome::deny(Reason::NoMatchingGrant)
    }
}

/// A grant applies when its subject is one of the caller's subjects and its
/// scope is one of the chain's. Inheritance can only ever ADD statements: a
/// leaf grant never leaks upward or sideways because a sibling's chain never
/// contains that leaf.
pub fn applicable_grants<'a>(
    grants: &'a [ResolvedGrant],
    subjects: &[SubjectEntry],
    scope_chain: &[ScopeEntry],
) -> Vec<&'a ResolvedGrant> {
    grants
        .iter()
        .filter(|grant| {
            let (Some(subject), Some(scope)) = (grant.subject.valid(), grant.scope.valid()) else {
                return false;
            };
            subjects
                .iter()
                .any(|candidate| candidate.valid() == Some(subject))
                && scope_chain
                    .iter()
                    .any(|candidate| candidate.valid() == Some(scope))
        })
        .collect()
}

fn has(grants: &[&ResolvedGrant], effect: Effect, query: &Query<'_>) -> bool {
    grants.iter().any(|grant| {
        grant
            .statements
            .iter()
            .any(|statement| statement_fires(statement, grant, effect, query))
    })
}

fn statement_fires(
    statement: &Statement,
    grant: &ResolvedGrant,
    effect: Effect,
    query: &Query<'_>,
) -> bool {
    if statement.effect != effect {
        return false;
    }
    if !action_ok(statement, effect, query) {
        return false;
    }
    if !statement
        .resources
        .iter()
        .any(|pattern| resource_matches(pattern, query.resource))
    {
        return false;
    }
    let verdict = conditions_verdict(statement, grant, effect, query);
    match effect {
        Effect::Allow => verdict == ConditionsVerdict::Match,
        Effect::Deny => verdict != ConditionsVerdict::NoMatch,
        Effect::Malformed => false,
    }
}

/// Action matching resolves asymmetrically for the same reason conditions do:
/// an allow needs a definitive match, a deny also fires on an unresolvable
/// ancestry rather than quietly ceasing to cover the leaves it names.
fn action_ok(statement: &Statement, effect: Effect, query: &Query<'_>) -> bool {
    let verdict = statement
        .actions
        .iter()
        .fold(ActionMatch::NoMatch, |best, candidate| {
            if best == ActionMatch::Match {
                best
            } else {
                best.or_stronger(match_action(
                    candidate,
                    query.action,
                    query.action_derivation,
                ))
            }
        });
    match effect {
        Effect::Allow => verdict == ActionMatch::Match,
        Effect::Deny => verdict != ActionMatch::NoMatch,
        Effect::Malformed => false,
    }
}

/// Bounds narrow ALLOW statements only: ANDing a bound onto a deny would make
/// it fire less often, widening access — the one direction this engine never
/// fails in.
fn conditions_verdict(
    statement: &Statement,
    grant: &ResolvedGrant,
    effect: Effect,
    query: &Query<'_>,
) -> ConditionsVerdict {
    let bounds = match effect {
        Effect::Allow => grant.bounds.as_ref().filter(|bounds| !bounds.is_empty()),
        Effect::Deny | Effect::Malformed => None,
    };
    let Some(bounds) = bounds else {
        return eval_conditions(
            statement.conditions.as_ref(),
            query.context,
            query.condition_keys,
        );
    };
    // A bound the write path would have refused could not have been stored by
    // a conforming host, so it never narrows an allow into existence here.
    // There is no write boundary in-process to catch it earlier.
    if crate::validate::validate_conditions(&Value::Array(bounds.clone()), query.condition_keys)
        .is_err()
    {
        return ConditionsVerdict::Unmatchable;
    }
    let mut combined: Vec<Value> = match statement.conditions.as_ref() {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(existing)) => existing.clone(),
        // Bounds can only narrow a list of conditions; a statement whose own
        // conditions are not a list is unreadable, never widened into one.
        Some(_) => return ConditionsVerdict::Unmatchable,
    };
    combined.extend(bounds.iter().cloned());
    eval_conditions(
        Some(&Value::Array(combined)),
        query.context,
        query.condition_keys,
    )
}
