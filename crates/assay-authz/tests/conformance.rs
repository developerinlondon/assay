//! The backend flip contract, run against this engine. Every case is decided
//! twice — once by the pure evaluator over the raw grant universe, once by
//! the composed engine with synthesizer grants kept separate — and both must
//! agree with the fixture.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use assay_authz::action::ActionDerivation;
use assay_authz::condition::{
    ConditionContext, ConditionKeys, builtin_context_entries, resolve_condition_keys,
};
use assay_authz::engine::{CheckOptions, Engine, EngineConfig};
use assay_authz::evaluate::{Decision, EvaluateInput, Query, evaluate};
use assay_authz::model::{ResolvedGrant, ScopeEntry, SubjectEntry};
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SuiteFile {
    suite: String,
    #[serde(default)]
    condition_keys: Option<ConditionKeys>,
    #[serde(default)]
    scope_kinds: Option<Vec<String>>,
    #[serde(default)]
    action_derivation: Option<BTreeMap<String, String>>,
    cases: Vec<RawCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCase {
    name: String,
    #[serde(default)]
    condition_keys: Option<ConditionKeys>,
    #[serde(default)]
    scope_kinds: Option<Vec<String>>,
    #[serde(default)]
    action_derivation: Option<BTreeMap<String, String>>,
    #[serde(default)]
    grants: Vec<ResolvedGrant>,
    #[serde(default)]
    synthesized_grants: Vec<ResolvedGrant>,
    check: RawCheck,
    expect: String,
    #[serde(default = "yes")]
    storable: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCheck {
    #[serde(default)]
    subjects: Vec<SubjectEntry>,
    action: String,
    resource: String,
    #[serde(default)]
    scope_chain: Vec<ScopeEntry>,
    #[serde(default)]
    context: ConditionContext,
    #[serde(default)]
    source_ip: Option<String>,
    #[serde(default)]
    now: Option<String>,
}

fn yes() -> bool {
    true
}

struct Case {
    suite: String,
    name: String,
    condition_keys: ConditionKeys,
    scope_kinds: Vec<String>,
    derivation: ActionDerivation,
    grants: Vec<ResolvedGrant>,
    synthesized_grants: Vec<ResolvedGrant>,
    check: RawCheck,
    expect: Decision,
    storable: bool,
}

impl Case {
    fn label(&self) -> String {
        format!("{}: {}", self.suite, self.name)
    }

    /// Absent when the case declares no derivation, so those cases exercise
    /// the exact-equality path rather than a lookup that always misses.
    fn derivation(&self) -> Option<&ActionDerivation> {
        (!self.derivation.is_empty()).then_some(&self.derivation)
    }

    fn now(&self) -> DateTime<Utc> {
        match self.check.now.as_deref() {
            Some(raw) => DateTime::parse_from_rfc3339(raw)
                .unwrap_or_else(|error| panic!("fixture `now` is not RFC 3339: {raw} ({error})"))
                .with_timezone(&Utc),
            None => Utc::now(),
        }
    }

    fn context(&self) -> ConditionContext {
        let mut context = self.check.context.clone();
        context.extend(builtin_context_entries(
            self.now(),
            self.check.source_ip.as_deref(),
        ));
        context
    }
}

fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("conformance/cases")
}

fn load_cases() -> Vec<Case> {
    let dir = cases_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .map(|entry| entry.expect("unreadable dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();

    let mut out = Vec::new();
    for path in files {
        let raw = std::fs::read_to_string(&path).expect("fixture is unreadable");
        let suite: SuiteFile = serde_json::from_str(&raw)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        for case in suite.cases {
            out.push(Case {
                suite: suite.suite.clone(),
                condition_keys: case
                    .condition_keys
                    .or_else(|| suite.condition_keys.clone())
                    .unwrap_or_default(),
                scope_kinds: case
                    .scope_kinds
                    .or_else(|| suite.scope_kinds.clone())
                    .unwrap_or_default(),
                derivation: ActionDerivation(
                    case.action_derivation
                        .or_else(|| suite.action_derivation.clone())
                        .unwrap_or_default(),
                ),
                name: case.name,
                grants: case.grants,
                synthesized_grants: case.synthesized_grants,
                expect: match case.expect.as_str() {
                    "allow" => Decision::Allow,
                    "deny" => Decision::Deny,
                    other => panic!("fixture expects an unknown decision: {other}"),
                },
                check: case.check,
                storable: case.storable,
            });
        }
    }
    out
}

/// The raw grant universe, exactly as the pure reference evaluator sees it.
fn pure_decision(case: &Case) -> Decision {
    let mut grants = case.grants.clone();
    grants.extend(case.synthesized_grants.iter().cloned());
    let context = case.context();
    let keys = resolve_condition_keys(&case.condition_keys);
    evaluate(&EvaluateInput {
        grants: &grants,
        subjects: &case.check.subjects,
        scope_chain: &case.check.scope_chain,
        scope_kinds: Some(&case.scope_kinds),
        query: Query {
            action: &case.check.action,
            resource: &case.check.resource,
            context: &context,
            condition_keys: &keys,
            action_derivation: case.derivation(),
        },
    })
    .decision
}

fn engine_for(case: &Case) -> Engine {
    Engine::new(EngineConfig {
        grants: case.grants.clone(),
        synthesized_grants: case.synthesized_grants.clone(),
        condition_keys: case.condition_keys.clone(),
        scope_kinds: Some(case.scope_kinds.clone()),
        action_derivation: case.derivation().cloned(),
        ..Default::default()
    })
}

fn engine_decision(case: &Case) -> Decision {
    engine_for(case)
        .check(
            &case.check.subjects,
            &case.check.action,
            &case.check.resource,
            &CheckOptions {
                scope_chain: Some(case.check.scope_chain.clone()),
                context: case.check.context.clone(),
                source_ip: case.check.source_ip.clone(),
                now: Some(case.now()),
                bypass: false,
            },
        )
        .decision
}

#[test]
fn the_fixture_suite_is_non_trivial() {
    let cases = load_cases();
    assert!(
        cases.len() >= 140,
        "expected the vendored fixture set, found {} cases",
        cases.len()
    );
    let suites: Vec<&str> = cases.iter().map(|case| case.suite.as_str()).collect();
    for expected in [
        "resource-matching",
        "deny-wins",
        "scope-chain",
        "conditions",
        "asymmetric-fail-closed",
        "role-synthesis",
        "action-derivation",
        "set-membership",
        "grant-bounds",
    ] {
        assert!(suites.contains(&expected), "missing suite {expected}");
    }
}

fn assert_decides_every_case(label: &str, decide: impl Fn(&Case) -> Decision) {
    let cases = load_cases();
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|case| {
            let actual = decide(case);
            (actual != case.expect).then(|| {
                format!(
                    "{} — expected {}, got {}",
                    case.label(),
                    case.expect.as_str(),
                    actual.as_str()
                )
            })
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    println!("{label}: {} cases passed", cases.len());
}

#[test]
fn the_pure_evaluator_decides_every_case() {
    assert_decides_every_case("pure evaluator", pure_decision);
}

#[test]
fn the_composed_engine_decides_every_case() {
    assert_decides_every_case("composed engine", engine_decision);
}

/// A `storable: false` case names a conditions shape a storage layer must
/// refuse at rest. With no storage layer the in-process equivalent is
/// validation: the shape must never survive `Engine::validate`.
#[test]
fn unstorable_shapes_are_refused_at_validate_time() {
    let cases = load_cases();
    let unstorable: Vec<&Case> = cases.iter().filter(|case| !case.storable).collect();
    assert!(
        !unstorable.is_empty(),
        "the fixture set carries no unstorable cases — the loader is wrong"
    );
    for case in &unstorable {
        let engine = engine_for(case);
        let offending: Vec<&ResolvedGrant> = case
            .grants
            .iter()
            .filter(|grant| {
                grant.statements.iter().any(|statement| {
                    statement
                        .conditions
                        .as_ref()
                        .is_some_and(|conditions| !is_condition_list(conditions))
                })
            })
            .collect();
        assert!(
            !offending.is_empty(),
            "{}: no grant carries the malformed shape the case names",
            case.label()
        );
        for grant in offending {
            let statements = serde_json::to_value(&grant.statements).expect("statements serialize");
            let error = engine.validate(&statements).expect_err(&format!(
                "{}: validate accepted an unstorable shape",
                case.label()
            ));
            assert!(
                error.contains("condition"),
                "{}: unexpected rejection — {error}",
                case.label()
            );
        }
    }
    println!(
        "unstorable shapes: {} cases refused at validate time",
        unstorable.len()
    );
}

fn is_condition_list(conditions: &serde_json::Value) -> bool {
    conditions
        .as_array()
        .is_some_and(|list| list.iter().all(serde_json::Value::is_object))
}
