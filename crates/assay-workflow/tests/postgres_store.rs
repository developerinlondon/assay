/// PostgresStore integration tests.
///
/// These tests spin up a real Postgres container via testcontainers
/// and run the full store contract against it. Requires Docker.
///
/// Run with: cargo test -p assay-workflow --test postgres_store
///
/// Skipped automatically when Docker is not available (e.g. macOS CI).
use assay_workflow::store::postgres::PostgresStore;
use assay_workflow::store::WorkflowStore;
use assay_workflow::types::*;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

fn docker_available() -> bool {
    std::process::Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Skip test if Docker is not running. Returns None to signal skip.
async fn create_store() -> Option<(PostgresStore, testcontainers::ContainerAsync<Postgres>)> {
    if !docker_available() {
        eprintln!("Skipping: Docker not available");
        return None;
    }
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let store = PostgresStore::new(&url).await.unwrap();
    Some((store, container))
}

/// Macro to skip tests when Docker is unavailable.
macro_rules! require_docker {
    ($store:ident, $container:ident) => {
        let Some(($store, $container)) = create_store().await else {
            return;
        };
        // Keep container alive for the test duration
        let _ = &$container;
    };
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
        search_attributes: None,
        created_at: ts,
        updated_at: ts,
        completed_at: None,
    }
}

#[tokio::test]
async fn pg_workflow_create_and_get() {
    require_docker!(store, _container);
    let wf = make_workflow("pg-wf-1", "IngestData");

    store.create_workflow(&wf).await.unwrap();
    let fetched = store.get_workflow("pg-wf-1").await.unwrap().unwrap();

    assert_eq!(fetched.id, "pg-wf-1");
    assert_eq!(fetched.namespace, "main");
    assert_eq!(fetched.workflow_type, "IngestData");
    assert_eq!(fetched.status, "PENDING");
}

#[tokio::test]
async fn pg_workflow_list_by_namespace() {
    require_docker!(store, _container);

    store
        .create_workflow(&make_workflow("pg-wf-1", "TypeA"))
        .await
        .unwrap();

    // Create a workflow in a different namespace
    store.create_namespace("staging").await.unwrap();
    let mut wf2 = make_workflow("pg-wf-2", "TypeB");
    wf2.namespace = "staging".to_string();
    store.create_workflow(&wf2).await.unwrap();

    // List main — should only see wf-1
    let main_wfs = store
        .list_workflows("main", None, None, None, 100, 0)
        .await
        .unwrap();
    assert_eq!(main_wfs.len(), 1);
    assert_eq!(main_wfs[0].id, "pg-wf-1");

    // List staging — should only see wf-2
    let staging_wfs = store
        .list_workflows("staging", None, None, None, 100, 0)
        .await
        .unwrap();
    assert_eq!(staging_wfs.len(), 1);
    assert_eq!(staging_wfs[0].id, "pg-wf-2");
}

#[tokio::test]
async fn pg_workflow_claim_and_status() {
    require_docker!(store, _container);
    store
        .create_workflow(&make_workflow("pg-wf-1", "TypeA"))
        .await
        .unwrap();

    let claimed = store.claim_workflow("pg-wf-1", "worker-1").await.unwrap();
    assert!(claimed);

    // Second claim fails
    let claimed_again = store.claim_workflow("pg-wf-1", "worker-2").await.unwrap();
    assert!(!claimed_again);

    let wf = store.get_workflow("pg-wf-1").await.unwrap().unwrap();
    assert_eq!(wf.status, "RUNNING");
    assert_eq!(wf.claimed_by.as_deref(), Some("worker-1"));

    // Complete
    store
        .update_workflow_status("pg-wf-1", WorkflowStatus::Completed, Some(r#"{"ok":true}"#), None)
        .await
        .unwrap();
    let wf = store.get_workflow("pg-wf-1").await.unwrap().unwrap();
    assert_eq!(wf.status, "COMPLETED");
    assert!(wf.completed_at.is_some());
}

#[tokio::test]
async fn pg_activity_claim_concurrent() {
    require_docker!(store, _container);
    store
        .create_workflow(&make_workflow("pg-wf-1", "TypeA"))
        .await
        .unwrap();

    let ts = now();
    store
        .create_activity(&WorkflowActivity {
            id: None,
            workflow_id: "pg-wf-1".to_string(),
            seq: 1,
            name: "fetch_data".to_string(),
            task_queue: "main".to_string(),
            input: None,
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

    // Two workers try to claim — only one should succeed
    // (FOR UPDATE SKIP LOCKED prevents contention)
    let claim1 = store.claim_activity("main", "worker-1").await.unwrap();
    let claim2 = store.claim_activity("main", "worker-2").await.unwrap();

    assert!(claim1.is_some());
    assert!(claim2.is_none()); // Already claimed by worker-1
}

#[tokio::test]
async fn pg_events_and_signals() {
    require_docker!(store, _container);
    store
        .create_workflow(&make_workflow("pg-wf-1", "TypeA"))
        .await
        .unwrap();

    let ts = now();
    store
        .append_event(&WorkflowEvent {
            id: None,
            workflow_id: "pg-wf-1".to_string(),
            seq: 1,
            event_type: "WorkflowStarted".to_string(),
            payload: Some(r#"{"input":"data"}"#.to_string()),
            timestamp: ts,
        })
        .await
        .unwrap();

    let events = store.list_events("pg-wf-1").await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "WorkflowStarted");

    let count = store.get_event_count("pg-wf-1").await.unwrap();
    assert_eq!(count, 1);

    // Signal
    store
        .send_signal(&WorkflowSignal {
            id: None,
            workflow_id: "pg-wf-1".to_string(),
            name: "approval".to_string(),
            payload: Some(r#"{"approved":true}"#.to_string()),
            consumed: false,
            received_at: ts,
        })
        .await
        .unwrap();

    let consumed = store.consume_signals("pg-wf-1", "approval").await.unwrap();
    assert_eq!(consumed.len(), 1);

    let consumed_again = store.consume_signals("pg-wf-1", "approval").await.unwrap();
    assert!(consumed_again.is_empty());
}

#[tokio::test]
async fn pg_timers() {
    require_docker!(store, _container);
    store
        .create_workflow(&make_workflow("pg-wf-1", "TypeA"))
        .await
        .unwrap();

    let past = now() - 10.0;
    let future = now() + 3600.0;

    store
        .create_timer(&WorkflowTimer {
            id: None,
            workflow_id: "pg-wf-1".to_string(),
            seq: 1,
            fire_at: past,
            fired: false,
        })
        .await
        .unwrap();

    store
        .create_timer(&WorkflowTimer {
            id: None,
            workflow_id: "pg-wf-1".to_string(),
            seq: 2,
            fire_at: future,
            fired: false,
        })
        .await
        .unwrap();

    let fired = store.fire_due_timers(now()).await.unwrap();
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].seq, 1);

    let fired_again = store.fire_due_timers(now()).await.unwrap();
    assert!(fired_again.is_empty());
}

#[tokio::test]
async fn pg_schedules() {
    require_docker!(store, _container);

    store
        .create_schedule(&WorkflowSchedule {
            name: "hourly".to_string(),
            namespace: "main".to_string(),
            workflow_type: "IngestData".to_string(),
            cron_expr: "0 * * * *".to_string(),
            timezone: "UTC".to_string(),
            input: None,
            task_queue: "main".to_string(),
            overlap_policy: "skip".to_string(),
            paused: false,
            last_run_at: None,
            next_run_at: None,
            last_workflow_id: None,
            created_at: now(),
        })
        .await
        .unwrap();

    let sched = store.get_schedule("main", "hourly").await.unwrap().unwrap();
    assert_eq!(sched.workflow_type, "IngestData");

    let all = store.list_schedules("main").await.unwrap();
    assert_eq!(all.len(), 1);

    let deleted = store.delete_schedule("main", "hourly").await.unwrap();
    assert!(deleted);
}

#[tokio::test]
async fn pg_namespace_stats() {
    require_docker!(store, _container);

    store
        .create_workflow(&make_workflow("pg-wf-1", "TypeA"))
        .await
        .unwrap();
    store
        .update_workflow_status("pg-wf-1", WorkflowStatus::Running, None, None)
        .await
        .unwrap();

    let stats = store.get_namespace_stats("main").await.unwrap();
    assert_eq!(stats.total_workflows, 1);
    assert_eq!(stats.running, 1);
    assert_eq!(stats.pending, 0);
}

#[tokio::test]
async fn pg_leader_lock() {
    require_docker!(store, _container);

    // Advisory lock should succeed — at least one attempt should work
    let acquired = store.try_acquire_leader_lock().await.unwrap();
    assert!(acquired);
}

#[tokio::test]
async fn pg_workers() {
    require_docker!(store, _container);
    let ts = now();

    store
        .register_worker(&WorkflowWorker {
            id: "w-pg-1".to_string(),
            namespace: "main".to_string(),
            identity: "pod-1".to_string(),
            task_queue: "main".to_string(),
            workflows: None,
            activities: None,
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

    store.heartbeat_worker("w-pg-1", ts + 30.0).await.unwrap();

    let removed = store.remove_dead_workers(ts + 29.0).await.unwrap();
    assert!(removed.is_empty());

    let removed = store.remove_dead_workers(ts + 31.0).await.unwrap();
    assert_eq!(removed.len(), 1);
}
