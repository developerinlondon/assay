//! Activity settlement is all-or-nothing.
//!
//! The defect these cover: an activity row reaching COMPLETED while the
//! workflow's history never received the matching event and no workflow task
//! was enqueued. The workflow then replays a history in which the activity is
//! still pending and never runs again.

use assay_workflow::health::reconcile_settled_activities;
use assay_workflow::types::*;
use assay_workflow::{SqliteStore, WorkflowStore};

const QUEUE: &str = "settle-q";

async fn seed() -> (SqliteStore, i64) {
    let store = SqliteStore::new("sqlite::memory:").await.unwrap();
    store
        .create_workflow(&WorkflowRecord {
            id: "wf-settle".to_string(),
            namespace: "main".to_string(),
            run_id: "run-1".to_string(),
            workflow_type: "TestWorkflow".to_string(),
            task_queue: QUEUE.to_string(),
            status: "RUNNING".to_string(),
            input: None,
            result: None,
            error: None,
            parent_id: None,
            claimed_by: None,
            search_attributes: None,
            archived_at: None,
            archive_uri: None,
            created_at: 1.0,
            updated_at: 1.0,
            completed_at: None,
        })
        .await
        .unwrap();
    let activity_id = store
        .create_activity(&WorkflowActivity {
            id: None,
            workflow_id: "wf-settle".to_string(),
            seq: 1,
            name: "verify".to_string(),
            task_queue: QUEUE.to_string(),
            input: None,
            status: "RUNNING".to_string(),
            result: None,
            error: None,
            attempt: 1,
            max_attempts: 3,
            initial_interval_secs: 1.0,
            backoff_coefficient: 2.0,
            start_to_close_secs: 300.0,
            heartbeat_timeout_secs: None,
            claimed_by: Some("w-1".to_string()),
            scheduled_at: 1.0,
            started_at: Some(1.0),
            completed_at: None,
            last_heartbeat: None,
        })
        .await
        .unwrap();
    (store, activity_id)
}

fn settlement<'a>(activity_id: i64, payload: &'a str) -> ActivitySettlement<'a> {
    ActivitySettlement {
        activity_id,
        workflow_id: "wf-settle",
        result: Some(r#"{"outcome":"repair"}"#),
        error: None,
        failed: false,
        event_type: "ActivityCompleted",
        payload,
        now: 2.0,
    }
}

/// Make every history append fail, standing in for the I/O failure that
/// produced the reported half-settled rows.
async fn block_event_writes(store: &SqliteStore) {
    sqlx::query(
        "CREATE TRIGGER workflow.block_events BEFORE INSERT ON events
         BEGIN SELECT RAISE(ABORT, 'injected write failure'); END",
    )
    .execute(store.pool())
    .await
    .unwrap();
}

fn payload_for(activity_id: i64) -> String {
    serde_json::json!({
        "activity_id": activity_id,
        "activity_seq": 1,
        "name": "verify",
        "result": {"outcome": "repair"},
        "error": serde_json::Value::Null,
    })
    .to_string()
}

/// Fail the history append and the activity must stay open: a caller that
/// sees an error has to be able to retry without the row already claiming
/// the work is done.
#[tokio::test]
async fn a_failed_history_append_rolls_back_the_status_write() {
    let (store, activity_id) = seed().await;
    block_event_writes(&store).await;

    let payload = payload_for(activity_id);
    let err = store
        .settle_activity(&settlement(activity_id, &payload))
        .await
        .expect_err("settle must fail while the history append fails");
    assert!(
        err.to_string().contains("injected write failure"),
        "unexpected error: {err}"
    );

    let act = store.get_activity(activity_id).await.unwrap().unwrap();
    assert_eq!(act.status, "RUNNING", "status write must have rolled back");
    assert_eq!(store.get_event_count("wf-settle").await.unwrap(), 0);
    assert!(
        store
            .claim_workflow_task(QUEUE, "w-1")
            .await
            .unwrap()
            .is_none(),
        "workflow must not be armed when the settle failed"
    );

    sqlx::query("DROP TRIGGER workflow.block_events")
        .execute(store.pool())
        .await
        .unwrap();
    let outcome = store
        .settle_activity(&settlement(activity_id, &payload))
        .await
        .unwrap();

    assert_eq!(outcome, SettleOutcome::Settled);
    let act = store.get_activity(activity_id).await.unwrap().unwrap();
    assert_eq!(act.status, "COMPLETED");
    assert_eq!(act.result.as_deref(), Some(r#"{"outcome":"repair"}"#));
    let events = store.list_events("wf-settle").await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "ActivityCompleted");
    assert!(
        store
            .claim_workflow_task(QUEUE, "w-1")
            .await
            .unwrap()
            .is_some(),
        "settling must enqueue the workflow task"
    );
}

/// Re-settling is the repair path for a workflow task lost after the event
/// landed: it re-arms dispatch, appends nothing, and leaves the recorded
/// result alone.
#[tokio::test]
async fn re_settling_rearms_dispatch_without_duplicating_the_event() {
    let (store, activity_id) = seed().await;
    let payload = payload_for(activity_id);
    store
        .settle_activity(&settlement(activity_id, &payload))
        .await
        .unwrap();
    // A worker claims, clearing needs_dispatch, then loses the task.
    store.claim_workflow_task(QUEUE, "w-1").await.unwrap();
    store
        .release_workflow_task("wf-settle", "w-1")
        .await
        .unwrap();

    let mut second = settlement(activity_id, &payload);
    second.result = Some(r#"{"outcome":"overwritten"}"#);
    let outcome = store.settle_activity(&second).await.unwrap();

    assert_eq!(outcome, SettleOutcome::AlreadySettled);
    assert_eq!(store.get_event_count("wf-settle").await.unwrap(), 1);
    let act = store.get_activity(activity_id).await.unwrap().unwrap();
    assert_eq!(
        act.result.as_deref(),
        Some(r#"{"outcome":"repair"}"#),
        "the first settle owns the result the history already carries"
    );
    assert!(
        store
            .claim_workflow_task(QUEUE, "w-1")
            .await
            .unwrap()
            .is_some(),
        "re-settling must re-enqueue the workflow task"
    );
}

/// A workflow left half-settled by an older engine converges on the next
/// health pass instead of sitting RUNNING forever.
#[tokio::test]
async fn reconcile_converges_an_activity_settled_without_its_event() {
    let (store, activity_id) = seed().await;
    // The pre-transaction shape: status and result written, no event, no
    // workflow task.
    store
        .complete_activity(activity_id, Some(r#"{"outcome":"repair"}"#), None, false)
        .await
        .unwrap();
    assert_eq!(store.get_event_count("wf-settle").await.unwrap(), 0);

    let unsettled = store.list_unsettled_activities(10).await.unwrap();
    assert_eq!(unsettled.len(), 1);
    assert_eq!(unsettled[0].id, Some(activity_id));

    let repaired = reconcile_settled_activities(&store).await.unwrap();

    assert_eq!(repaired, 1);
    let events = store.list_events("wf-settle").await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "ActivityCompleted");
    let payload: serde_json::Value =
        serde_json::from_str(events[0].payload.as_deref().unwrap()).unwrap();
    assert_eq!(payload["activity_id"].as_i64(), Some(activity_id));
    assert_eq!(payload["result"]["outcome"], "repair");
    assert!(
        store
            .claim_workflow_task(QUEUE, "w-1")
            .await
            .unwrap()
            .is_some(),
        "reconcile must enqueue the workflow task"
    );
    assert!(
        store
            .list_unsettled_activities(10)
            .await
            .unwrap()
            .is_empty(),
        "a repaired activity must not be repaired twice"
    );
}

/// A signal that cannot be appended to history must not be recorded at all:
/// a stored signal the workflow never sees is a run that waits forever.
#[tokio::test]
async fn a_failed_history_append_rolls_back_the_signal() {
    let (store, _) = seed().await;
    block_event_writes(&store).await;

    let signal = WorkflowSignal {
        id: None,
        workflow_id: "wf-settle".to_string(),
        name: "approve".to_string(),
        payload: Some(r#"{"ok":true}"#.to_string()),
        consumed: false,
        received_at: 3.0,
    };
    store
        .deliver_signal(&signal, r#"{"signal":"approve"}"#)
        .await
        .expect_err("signal delivery must fail while the history append fails");

    assert!(
        store
            .consume_signals("wf-settle", "approve")
            .await
            .unwrap()
            .is_empty(),
        "the signal row must have rolled back with the event"
    );
    assert!(
        store
            .claim_workflow_task(QUEUE, "w-1")
            .await
            .unwrap()
            .is_none(),
        "workflow must not be armed for a signal that did not land"
    );
}

/// The reaper removes a silent worker's registration. Heartbeating an id
/// that no longer exists has to say so, or the worker keeps polling under an
/// id no queue dispatches to until someone restarts it.
#[tokio::test]
async fn heartbeat_reports_a_reaped_registration() {
    let (store, _) = seed().await;
    let worker = WorkflowWorker {
        id: "w-1".to_string(),
        namespace: "main".to_string(),
        identity: "test-worker".to_string(),
        task_queue: QUEUE.to_string(),
        workflows: None,
        activities: None,
        max_concurrent_workflows: 10,
        max_concurrent_activities: 10,
        active_tasks: 0,
        last_heartbeat: 10.0,
        registered_at: 10.0,
    };
    store.register_worker(&worker).await.unwrap();
    assert!(store.heartbeat_worker("w-1", 11.0).await.unwrap());

    store.remove_dead_workers(100.0).await.unwrap();

    assert!(
        !store.heartbeat_worker("w-1", 101.0).await.unwrap(),
        "a heartbeat for a reaped registration must report the row is gone"
    );
}
