//! Behaviour the golden fixtures do not reach: context values of every shape,
//! bounds the write path would refuse, and rows missing a required field.
//! Every verdict here was read off the reference implementation first.

use assay_authz::condition::{ConditionKeySpec, ConditionKeyType, ConditionKeys, format_number};
use assay_authz::engine::{CheckOptions, Engine, EngineConfig};
use assay_authz::evaluate::Decision;
use assay_authz::model::{ResolvedGrant, Scope, ScopeEntry, Subject, SubjectEntry};
use serde_json::{Value, json};

fn keys() -> ConditionKeys {
    ["app:Roles", "app:Flag", "app:Str"]
        .into_iter()
        .map(|key| {
            (
                key.to_string(),
                ConditionKeySpec {
                    key_type: ConditionKeyType::String,
                    ..Default::default()
                },
            )
        })
        .collect()
}

fn root() -> ScopeEntry {
    ScopeEntry::Valid(Scope {
        kind: "root".into(),
        id: "*".into(),
    })
}

fn alice() -> SubjectEntry {
    SubjectEntry::Valid(Subject {
        kind: "user".into(),
        id: "alice".into(),
    })
}

fn grant(effect: &str, conditions: Value, bounds: Option<Value>) -> ResolvedGrant {
    let mut raw = json!({
        "subject": { "kind": "user", "id": "alice" },
        "scope": { "kind": "root", "id": "*" },
        "statements": [{
            "effect": effect,
            "actions": ["docs.read"],
            "resources": ["*"],
            "conditions": conditions,
        }],
    });
    if let Some(bounds) = bounds {
        raw["bounds"] = bounds;
    }
    serde_json::from_value(raw).expect("grant deserializes")
}

fn decide_with(grants: Vec<ResolvedGrant>, context: Value) -> Decision {
    let engine = Engine::new(EngineConfig {
        grants,
        condition_keys: keys(),
        scope_kinds: Some(vec!["root".into()]),
        default_scope_chain: vec![root()],
        ..Default::default()
    });
    engine
        .check(
            &[alice()],
            "docs.read",
            "doc:1",
            &CheckOptions {
                context: serde_json::from_value(context).expect("context deserializes"),
                now: Some(assay_authz::parse_rfc3339("2026-07-14T12:30:00Z").unwrap()),
                ..Default::default()
            },
        )
        .decision
}

fn allow_with(conditions: Value, context: Value) -> Decision {
    decide_with(vec![grant("allow", conditions, None)], context)
}

#[test]
fn an_empty_list_context_value_decides_rather_than_aborting() {
    let not_in = json!([{ "operator": "StringNotIn", "key": "app:Roles", "values": ["admin"] }]);
    let is_in = json!([{ "operator": "StringIn", "key": "app:Roles", "values": ["admin"] }]);
    let empty = json!({ "app:Roles": [] });

    // The reference holds no admin role, so StringNotIn is satisfied and a
    // deny gated on it fires.
    assert_eq!(
        decide_with(vec![grant("deny", not_in.clone(), None)], empty.clone()),
        Decision::Deny
    );
    assert_eq!(allow_with(not_in, empty.clone()), Decision::Allow);
    assert_eq!(allow_with(is_in, empty), Decision::Deny);
}

#[test]
fn an_empty_object_context_value_reads_as_an_empty_list() {
    let not_in = json!([{ "operator": "StringNotIn", "key": "app:Roles", "values": ["admin"] }]);
    assert_eq!(
        allow_with(not_in, json!({ "app:Roles": {} })),
        Decision::Allow
    );
}

#[test]
fn a_boolean_context_value_never_equals_a_string_but_still_renders() {
    let equals = json!([{ "operator": "StringEquals", "key": "app:Flag", "value": "true" }]);
    let not_equals = json!([{ "operator": "StringNotEquals", "key": "app:Flag", "value": "true" }]);
    let like = json!([{ "operator": "StringLike", "key": "app:Flag", "value": "tr*" }]);
    let is_in = json!([{ "operator": "StringIn", "key": "app:Flag", "values": ["true"] }]);
    let flag = json!({ "app:Flag": true });

    assert_eq!(allow_with(equals, flag.clone()), Decision::Deny);
    assert_eq!(allow_with(not_equals, flag.clone()), Decision::Allow);
    assert_eq!(allow_with(like, flag.clone()), Decision::Allow);
    assert_eq!(allow_with(is_in, flag), Decision::Allow);
}

#[test]
fn a_number_under_a_string_key_compares_as_the_reference_renders_it() {
    let equals = json!([{ "operator": "StringEquals", "key": "app:Str", "value": "12" }]);
    let like = json!([{ "operator": "StringLike", "key": "app:Str", "value": "1*" }]);
    let is_in = json!([{ "operator": "StringIn", "key": "app:Str", "values": ["12"] }]);
    let number = json!({ "app:Str": 12 });

    assert_eq!(allow_with(equals, number.clone()), Decision::Deny);
    assert_eq!(allow_with(like, number.clone()), Decision::Allow);
    assert_eq!(allow_with(is_in, number), Decision::Allow);
}

#[test]
fn an_unreadable_context_value_decides_rather_than_aborting() {
    let not_equals = json!([{ "operator": "StringNotEquals", "key": "app:Str", "value": "x" }]);
    let like = json!([{ "operator": "StringLike", "key": "app:Str", "value": "x*" }]);

    assert_eq!(
        allow_with(not_equals.clone(), json!({ "app:Str": { "a": 1 } })),
        Decision::Allow
    );
    assert_eq!(
        allow_with(like, json!({ "app:Str": { "a": 1 } })),
        Decision::Deny
    );
    assert_eq!(
        allow_with(not_equals, json!({ "app:Str": null })),
        Decision::Allow
    );
}

#[test]
fn a_bound_the_write_path_would_refuse_never_narrows_an_allow_into_existence() {
    let timezoned = json!([{
        "operator": "DateLessThan", "key": "request:Time", "value": "2027-01-01T00:00:00Z"
    }]);
    let bare = json!([{
        "operator": "DateLessThan", "key": "request:Time", "value": "2027-01-01T00:00:00"
    }]);
    let leap = json!([{
        "operator": "DateLessThan", "key": "request:Time", "value": "2026-06-30T23:59:60Z"
    }]);

    assert_eq!(
        decide_with(
            vec![grant("allow", Value::Null, Some(timezoned))],
            json!({})
        ),
        Decision::Allow
    );
    assert_eq!(
        decide_with(vec![grant("allow", Value::Null, Some(bare))], json!({})),
        Decision::Deny
    );
    assert_eq!(
        decide_with(vec![grant("allow", Value::Null, Some(leap))], json!({})),
        Decision::Deny
    );
}

#[test]
fn validate_bounds_refuses_what_the_evaluator_refuses() {
    let engine = Engine::new(EngineConfig {
        condition_keys: keys(),
        ..Default::default()
    });
    let bare = json!([{
        "operator": "DateLessThan", "key": "request:Time", "value": "2027-01-01T00:00:00"
    }]);
    let error = engine.validate_bounds(&bare).expect_err("must be refused");
    assert!(error.contains("timezone"), "unexpected error: {error}");
    assert!(
        engine
            .validate_bounds(&json!([{
                "operator": "DateLessThan", "key": "request:Time", "value": "2027-01-01T00:00:00Z"
            }]))
            .is_ok()
    );
}

#[test]
fn a_grant_missing_its_subject_is_dropped_not_fatal() {
    let headless: ResolvedGrant = serde_json::from_value(json!({
        "scope": { "kind": "root", "id": "*" },
        "statements": [{ "effect": "allow", "actions": ["docs.read"], "resources": ["*"] }],
    }))
    .expect("a grant missing its subject still loads");
    assert!(headless.subject.valid().is_none());
    assert_eq!(decide_with(vec![headless], json!({})), Decision::Deny);
}

#[test]
fn a_statement_missing_its_effect_is_skipped_not_fatal() {
    let mixed: ResolvedGrant = serde_json::from_value(json!({
        "subject": { "kind": "user", "id": "alice" },
        "scope": { "kind": "root", "id": "*" },
        "statements": [
            { "actions": ["docs.read"], "resources": ["*"] },
            { "effect": "allow", "actions": ["docs.read"], "resources": ["*"] },
        ],
    }))
    .expect("a statement missing its effect still loads");
    assert_eq!(decide_with(vec![mixed], json!({})), Decision::Allow);

    let only_headless: ResolvedGrant = serde_json::from_value(json!({
        "subject": { "kind": "user", "id": "alice" },
        "scope": { "kind": "root", "id": "*" },
        "statements": [{ "actions": ["docs.read"], "resources": ["*"] }],
    }))
    .expect("loads");
    assert_eq!(decide_with(vec![only_headless], json!({})), Decision::Deny);
}

#[test]
fn a_grant_authored_in_snake_case_keeps_its_identity() {
    let grant: ResolvedGrant = serde_json::from_value(json!({
        "policy_id": "p1",
        "policy_name": "reader",
        "subject": { "kind": "user", "id": "alice" },
        "scope": { "kind": "root", "id": "*" },
        "statements": [],
    }))
    .expect("loads");
    assert_eq!(grant.policy_id, "p1");
    assert_eq!(grant.policy_name, "reader");
}

#[test]
fn numbers_render_as_javascript_renders_them() {
    for (value, expected) in [
        (12.0, "12"),
        (0.0, "0"),
        (-0.0, "0"),
        (0.1, "0.1"),
        (-3.25, "-3.25"),
        (1e20, "100000000000000000000"),
        (1e21, "1e+21"),
        (1.5e21, "1.5e+21"),
        (1e-6, "0.000001"),
        (1e-7, "1e-7"),
        (1.5e-7, "1.5e-7"),
        (1e300, "1e+300"),
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
        (f64::NAN, "NaN"),
    ] {
        assert_eq!(format_number(value), expected, "rendering {value}");
    }
}
