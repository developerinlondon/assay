use assay::workflow::store::sqlite::SqliteStore;
use assay::workflow::store::WorkflowStore;
use assay::workflow::types::*;

async fn test_store() -> SqliteStore {
    SqliteStore::new("sqlite::memory:").await.unwrap()
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

fn make_workflow(id: &str, wf_type: &str) -> WorkflowRecord {
    let ts = now();
    WorkflowRecord {
        id: id.to_string(),
        namespace: "main".to_string(),
        run_id: format!("run-{id}"),
        workflow_type: wf_type.to_string(),
        task_queue: "main".to_string(),
        status: "PENDING".to_string(),
        input: Some(r#"{"key":"value"}"#.to_string()),
        result: None,
        error: None,
        parent_id: None,
        claimed_by: None,
        created_at: ts,
        updated_at: ts,
        completed_at: None,
    }
}

#[tokio::test]
async fn workflow_create_and_get() {
    let store = test_store().await;
    let wf = make_workflow("wf-1", "IngestData");

    store.create_workflow(&wf).await.unwrap();
    let fetched = store.get_workflow("wf-1").await.unwrap().unwrap();

    assert_eq!(fetched.id, "wf-1");
    assert_eq!(fetched.workflow_type, "IngestData");
    assert_eq!(fetched.status, "PENDING");
    assert_eq!(fetched.input.as_deref(), Some(r#"{"key":"value"}"#));
}

#[tokio::test]
async fn workflow_get_nonexistent() {
    let store = test_store().await;
    let result = store.get_workflow("nonexistent").await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn workflow_list_filter_by_status() {
    let store = test_store().await;
    store
        .create_workflow(&make_workflow("wf-1", "TypeA"))
        .await
        .unwrap();
    store
        .create_workflow(&make_workflow("wf-2", "TypeB"))
        .await
        .unwrap();

    store
        .update_workflow_status("wf-1", WorkflowStatus::Running, None, None)
        .await
        .unwrap();

    let running = store
        .list_workflows("main", Some(WorkflowStatus::Running), None, 100, 0)
        .await
        .unwrap();
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].id, "wf-1");

    let pending = store
        .list_workflows("main", Some(WorkflowStatus::Pending), None, 100, 0)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "wf-2");

    let all = store.list_workflows("main", None, None, 100, 0).await.unwrap();
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn workflow_claim() {
    let store = test_store().await;
    store
        .create_workflow(&make_workflow("wf-1", "TypeA"))
        .await
        .unwrap();

    let claimed = store.claim_workflow("wf-1", "worker-1").await.unwrap();
    assert!(claimed);

    // Second claim should fail
    let claimed_again = store.claim_workflow("wf-1", "worker-2").await.unwrap();
    assert!(!claimed_again);

    let wf = store.get_workflow("wf-1").await.unwrap().unwrap();
    assert_eq!(wf.claimed_by.as_deref(), Some("worker-1"));
    assert_eq!(wf.status, "RUNNING");
}

#[tokio::test]
async fn workflow_update_status_to_completed() {
    let store = test_store().await;
    store
        .create_workflow(&make_workflow("wf-1", "TypeA"))
        .await
        .unwrap();

    store
        .update_workflow_status("wf-1", WorkflowStatus::Completed, Some(r#"{"done":true}"#), None)
        .await
        .unwrap();

    let wf = store.get_workflow("wf-1").await.unwrap().unwrap();
    assert_eq!(wf.status, "COMPLETED");
    assert_eq!(wf.result.as_deref(), Some(r#"{"done":true}"#));
    assert!(wf.completed_at.is_some());
}

#[tokio::test]
async fn event_append_and_list() {
    let store = test_store().await;
    store
        .create_workflow(&make_workflow("wf-1", "TypeA"))
        .await
        .unwrap();

    let ts = now();
    store
        .append_event(&WorkflowEvent {
            id: None,
            workflow_id: "wf-1".to_string(),
            seq: 1,
            event_type: "WorkflowStarted".to_string(),
            payload: Some(r#"{"input":"data"}"#.to_string()),
            timestamp: ts,
        })
        .await
        .unwrap();

    store
        .append_event(&WorkflowEvent {
            id: None,
            workflow_id: "wf-1".to_string(),
            seq: 2,
            event_type: "ActivityScheduled".to_string(),
            payload: Some(r#"{"name":"fetch"}"#.to_string()),
            timestamp: ts + 1.0,
        })
        .await
        .unwrap();

    let events = store.list_events("wf-1").await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].seq, 1);
    assert_eq!(events[0].event_type, "WorkflowStarted");
    assert_eq!(events[1].seq, 2);

    let count = store.get_event_count("wf-1").await.unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn activity_create_claim_complete() {
    let store = test_store().await;
    store
        .create_workflow(&make_workflow("wf-1", "TypeA"))
        .await
        .unwrap();

    let ts = now();
    store
        .create_activity(&WorkflowActivity {
            id: None,
            workflow_id: "wf-1".to_string(),
            seq: 1,
            name: "fetch_data".to_string(),
            task_queue: "default".to_string(),
            input: Some(r#"{"url":"http://example.com"}"#.to_string()),
            status: "PENDING".to_string(),
            result: None,
            error: None,
            attempt: 1,
            max_attempts: 3,
            initial_interval_secs: 1.0,
            backoff_coefficient: 2.0,
            start_to_close_secs: 300.0,
            heartbeat_timeout_secs: None,
            claimed_by: None,
            scheduled_at: ts,
            started_at: None,
            completed_at: None,
            last_heartbeat: None,
        })
        .await
        .unwrap();

    // Claim from wrong queue returns None
    let wrong_queue = store.claim_activity("gpu", "worker-1").await.unwrap();
    assert!(wrong_queue.is_none());

    // Claim from correct queue
    let claimed = store.claim_activity("default", "worker-1").await.unwrap();
    assert!(claimed.is_some());
    let act = claimed.unwrap();
    assert_eq!(act.name, "fetch_data");
    assert_eq!(act.status, "RUNNING");

    // No more pending activities
    let next = store.claim_activity("default", "worker-2").await.unwrap();
    assert!(next.is_none());

    // Complete the activity
    store
        .complete_activity(act.id.unwrap(), Some(r#"{"rows":42}"#), None, false)
        .await
        .unwrap();
}

#[tokio::test]
async fn timer_create_and_fire() {
    let store = test_store().await;
    store
        .create_workflow(&make_workflow("wf-1", "TypeA"))
        .await
        .unwrap();

    let past = now() - 10.0;
    let future = now() + 3600.0;

    store
        .create_timer(&WorkflowTimer {
            id: None,
            workflow_id: "wf-1".to_string(),
            seq: 1,
            fire_at: past,
            fired: false,
        })
        .await
        .unwrap();

    store
        .create_timer(&WorkflowTimer {
            id: None,
            workflow_id: "wf-1".to_string(),
            seq: 2,
            fire_at: future,
            fired: false,
        })
        .await
        .unwrap();

    // Only the past timer should fire
    let fired = store.fire_due_timers(now()).await.unwrap();
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].seq, 1);
    assert!(fired[0].fired);

    // Firing again returns nothing (already fired)
    let fired_again = store.fire_due_timers(now()).await.unwrap();
    assert!(fired_again.is_empty());
}

#[tokio::test]
async fn signal_send_and_consume() {
    let store = test_store().await;
    store
        .create_workflow(&make_workflow("wf-1", "TypeA"))
        .await
        .unwrap();

    store
        .send_signal(&WorkflowSignal {
            id: None,
            workflow_id: "wf-1".to_string(),
            name: "approval".to_string(),
            payload: Some(r#"{"approved":true}"#.to_string()),
            consumed: false,
            received_at: now(),
        })
        .await
        .unwrap();

    store
        .send_signal(&WorkflowSignal {
            id: None,
            workflow_id: "wf-1".to_string(),
            name: "approval".to_string(),
            payload: Some(r#"{"approved":false}"#.to_string()),
            consumed: false,
            received_at: now(),
        })
        .await
        .unwrap();

    // Consume both signals
    let consumed = store.consume_signals("wf-1", "approval").await.unwrap();
    assert_eq!(consumed.len(), 2);

    // Already consumed — should return empty
    let consumed_again = store.consume_signals("wf-1", "approval").await.unwrap();
    assert!(consumed_again.is_empty());
}

#[tokio::test]
async fn schedule_crud() {
    let store = test_store().await;

    store
        .create_schedule(&WorkflowSchedule {
            name: "hourly-ingest".to_string(),
            namespace: "main".to_string(),
            workflow_type: "IngestData".to_string(),
            cron_expr: "0 * * * *".to_string(),
            input: None,
            task_queue: "main".to_string(),
            overlap_policy: "skip".to_string(),
            paused: false,
            last_run_at: None,
            next_run_at: Some(now() + 3600.0),
            last_workflow_id: None,
            created_at: now(),
        })
        .await
        .unwrap();

    let sched = store.get_schedule("main", "hourly-ingest").await.unwrap().unwrap();
    assert_eq!(sched.workflow_type, "IngestData");
    assert_eq!(sched.cron_expr, "0 * * * *");

    let all = store.list_schedules("main").await.unwrap();
    assert_eq!(all.len(), 1);

    store
        .update_schedule_last_run("main", "hourly-ingest", now(), now() + 3600.0, "wf-run-1")
        .await
        .unwrap();

    let updated = store.get_schedule("main", "hourly-ingest").await.unwrap().unwrap();
    assert!(updated.last_run_at.is_some());
    assert_eq!(updated.last_workflow_id.as_deref(), Some("wf-run-1"));

    let deleted = store.delete_schedule("main", "hourly-ingest").await.unwrap();
    assert!(deleted);

    let gone = store.get_schedule("main", "hourly-ingest").await.unwrap();
    assert!(gone.is_none());
}

#[tokio::test]
async fn worker_register_heartbeat_remove() {
    let store = test_store().await;
    let ts = now();

    store
        .register_worker(&WorkflowWorker {
            id: "w-1".to_string(),
            namespace: "main".to_string(),
            identity: "pipeline-pod-1".to_string(),
            task_queue: "main".to_string(),
            workflows: Some(r#"["IngestData"]"#.to_string()),
            activities: Some(r#"["fetch_data"]"#.to_string()),
            max_concurrent_workflows: 10,
            max_concurrent_activities: 20,
            active_tasks: 0,
            last_heartbeat: ts,
            registered_at: ts,
        })
        .await
        .unwrap();

    let workers = store.list_workers("main").await.unwrap();
    assert_eq!(workers.len(), 1);
    assert_eq!(workers[0].identity, "pipeline-pod-1");

    store.heartbeat_worker("w-1", ts + 30.0).await.unwrap();

    // Remove workers that haven't heartbeated since cutoff
    // w-1 heartbeated at ts+30, cutoff is ts+29 — worker is still alive (30 > 29)
    let removed = store.remove_dead_workers(ts + 29.0).await.unwrap();
    assert!(removed.is_empty());

    // cutoff at ts+31 — worker is dead (last heartbeat 30 < cutoff 31)
    let removed = store.remove_dead_workers(ts + 31.0).await.unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0], "w-1");

    let workers = store.list_workers("main").await.unwrap();
    assert!(workers.is_empty());
}
