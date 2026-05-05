//! Compile-time check that the trait surface is what we expect —
//! lets future refactors flag accidental API breaks at `cargo test`
//! time rather than by downstream crates failing to build.

use std::sync::Arc;

use assay_domain::events::{EngineEventBus, Event, EventFilter, NewEvent, Subsystem};

fn _is_dyn_compatible() {
    let _: Option<Arc<dyn EngineEventBus>> = None;
}

fn _filter_compiles() {
    let f = EventFilter {
        subsystems: vec![Subsystem::Workflow],
        kinds: vec!["workflow_created".to_string()],
        workflow_id: Some("wf-1".to_string()),
    };
    let e = Event {
        id: 1,
        ts: 0.0,
        namespace: "main".into(),
        subsystem: Subsystem::Workflow,
        kind: "workflow_created".into(),
        payload: serde_json::json!({ "workflow_id": "wf-1" }),
    };
    assert!(f.matches(&e));
}

fn _new_event_compiles() {
    let _ = NewEvent {
        namespace: "main",
        subsystem: Subsystem::Workflow,
        kind: "workflow_created",
        payload: serde_json::json!({}),
    };
}

fn _subsystem_round_trip() {
    assert_eq!(
        Subsystem::from_str(Subsystem::Workflow.as_str()),
        Subsystem::Workflow
    );
    assert_eq!(
        Subsystem::from_str(Subsystem::Auth.as_str()),
        Subsystem::Auth
    );
    assert_eq!(
        Subsystem::from_str(Subsystem::Secrets.as_str()),
        Subsystem::Secrets
    );
    assert_eq!(
        Subsystem::from_str(Subsystem::System.as_str()),
        Subsystem::System
    );
    assert_eq!(Subsystem::from_str("unknown_subsystem"), Subsystem::System);
}

#[test]
fn shapes_hold() {
    _is_dyn_compatible();
    _filter_compiles();
    _new_event_compiles();
    _subsystem_round_trip();
}
