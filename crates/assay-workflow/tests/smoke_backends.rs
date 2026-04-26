mod common;
use common::harness::{make_event, make_workflow};
use common::Backend;
use rstest::rstest;

// ── Helper: unique workflow id per test ───────────────────────────────────────

fn uid(prefix: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!("{prefix}-{t}")
}

// ── Task 3.1 (baseline) ───────────────────────────────────────────────────────

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn namespace_roundtrip(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    // "main" namespace is created during harness setup — it must appear in the list.
    let list = h.list_namespaces().await.unwrap();
    assert!(list.iter().any(|n| n.name == "main"));
}

// ── Task 3.2 — Namespaces ─────────────────────────────────────────────────────

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn namespace_delete_and_stats(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let ns = uid("ns-del");

    // Create
    h.create_namespace(&ns).await.unwrap();
    let list = h.list_namespaces().await.unwrap();
    assert!(list.iter().any(|n| n.name == ns), "namespace should exist after create");

    // Stats (empty)
    let stats = h.get_namespace_stats(&ns).await.unwrap();
    assert_eq!(stats.namespace, ns);
    assert_eq!(stats.total_workflows, 0);

    // Add a workflow so total_workflows > 0
    let wf = make_workflow(&uid("wf"), &ns, "main");
    h.create_workflow(&wf).await.unwrap();
    let stats2 = h.get_namespace_stats(&ns).await.unwrap();
    assert_eq!(stats2.total_workflows, 1);
    assert_eq!(stats2.pending, 1);

    // Delete
    let deleted = h.delete_namespace(&ns).await.unwrap();
    assert!(deleted, "delete should return true");
    let list2 = h.list_namespaces().await.unwrap();
    assert!(!list2.iter().any(|n| n.name == ns), "namespace should be gone");

    // Double-delete returns false
    let deleted2 = h.delete_namespace(&ns).await.unwrap();
    assert!(!deleted2, "second delete should return false");

    // 'main' is always available — delete must refuse.
    let main_del = h.delete_namespace("main").await.unwrap();
    assert!(!main_del, "cannot delete 'main'");
}

// ── Task 3.3 — Workflows ──────────────────────────────────────────────────────

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn workflow_create_get_roundtrip(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let id = uid("wf-cg");
    let wf = make_workflow(&id, "main", "test-queue");

    h.create_workflow(&wf).await.unwrap();

    let got = h.get_workflow(&id).await.unwrap().expect("workflow should exist");
    assert_eq!(got.id, id);
    assert_eq!(got.namespace, "main");
    assert_eq!(got.run_id, wf.run_id);
    assert_eq!(got.workflow_type, "test_wf");
    assert_eq!(got.task_queue, "test-queue");
    assert_eq!(got.status, "PENDING");

    // Missing id returns None
    let missing = h.get_workflow("nonexistent-id").await.unwrap();
    assert!(missing.is_none());
}

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn workflow_status_transitions(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let id = uid("wf-st");
    h.create_workflow(&make_workflow(&id, "main", "main")).await.unwrap();

    // PENDING → RUNNING
    h.update_workflow_status(&id, assay_domain::types::WorkflowStatus::Running, None, None)
        .await
        .unwrap();
    let wf = h.get_workflow(&id).await.unwrap().unwrap();
    assert_eq!(wf.status, "RUNNING");
    assert!(wf.completed_at.is_none(), "completed_at should be None while running");

    // RUNNING → COMPLETED (terminal)
    h.update_workflow_status(
        &id,
        assay_domain::types::WorkflowStatus::Completed,
        Some(r#"{"ok":true}"#),
        None,
    )
    .await
    .unwrap();
    let wf2 = h.get_workflow(&id).await.unwrap().unwrap();
    assert_eq!(wf2.status, "COMPLETED");
    assert!(wf2.completed_at.is_some(), "completed_at should be set for terminal status");
    assert_eq!(wf2.result.as_deref(), Some(r#"{"ok":true}"#));
}

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn workflow_claim(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let id = uid("wf-cl");
    h.create_workflow(&make_workflow(&id, "main", "main")).await.unwrap();

    // Worker A claims successfully
    let claimed_a = h.claim_workflow(&id, "worker-a").await.unwrap();
    assert!(claimed_a, "first claim should succeed");

    // Worker B fails to claim (already claimed)
    let claimed_b = h.claim_workflow(&id, "worker-b").await.unwrap();
    assert!(!claimed_b, "second claim should fail");

    // Verify the DB reflects worker-a
    let wf = h.get_workflow(&id).await.unwrap().unwrap();
    assert_eq!(wf.claimed_by.as_deref(), Some("worker-a"));
    assert_eq!(wf.status, "RUNNING");
}

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn workflow_list_with_filters(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let ns = uid("ns-lf");
    h.create_namespace(&ns).await.unwrap();

    let id1 = uid("wf-lf1");
    let id2 = uid("wf-lf2");

    let mut wf1 = make_workflow(&id1, &ns, "q1");
    wf1.workflow_type = "type-alpha".to_string();
    let mut wf2 = make_workflow(&id2, &ns, "q2");
    wf2.workflow_type = "type-beta".to_string();

    h.create_workflow(&wf1).await.unwrap();
    h.create_workflow(&wf2).await.unwrap();

    // List all in namespace
    let all = h.list_workflows(&ns, None, None, None, 100, 0).await.unwrap();
    assert_eq!(all.len(), 2);

    // Filter by workflow_type
    let alpha = h.list_workflows(&ns, None, Some("type-alpha"), None, 100, 0).await.unwrap();
    assert_eq!(alpha.len(), 1);
    assert_eq!(alpha[0].id, id1);

    // Filter by status (PENDING)
    let pending = h
        .list_workflows(&ns, Some(assay_domain::types::WorkflowStatus::Pending), None, None, 100, 0)
        .await
        .unwrap();
    assert_eq!(pending.len(), 2);

    // Update one to RUNNING, then filter
    h.update_workflow_status(&id1, assay_domain::types::WorkflowStatus::Running, None, None)
        .await
        .unwrap();
    let running = h
        .list_workflows(&ns, Some(assay_domain::types::WorkflowStatus::Running), None, None, 100, 0)
        .await
        .unwrap();
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].id, id1);
}

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn workflow_task_dispatch(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let id = uid("wf-td");
    let wf = make_workflow(&id, "main", "dispatch-queue");
    h.create_workflow(&wf).await.unwrap();

    // Nothing to claim before marking dispatchable
    let none = h.claim_workflow_task("dispatch-queue", "worker-1").await.unwrap();
    assert!(none.is_none(), "no task before mark_dispatchable");

    // Mark dispatchable
    h.mark_workflow_dispatchable(&id).await.unwrap();

    // Worker 1 claims it
    let claimed = h.claim_workflow_task("dispatch-queue", "worker-1").await.unwrap();
    assert!(claimed.is_some(), "should claim after mark_dispatchable");
    let claimed_wf = claimed.unwrap();
    assert_eq!(claimed_wf.id, id);

    // Worker 2 cannot claim the same (already claimed)
    let none2 = h.claim_workflow_task("dispatch-queue", "worker-2").await.unwrap();
    assert!(none2.is_none(), "second worker should not claim already-claimed task");

    // Release by worker 1
    h.release_workflow_task(&id, "worker-1").await.unwrap();

    // Now worker 2 can claim (but needs_dispatch was cleared — re-mark)
    h.mark_workflow_dispatchable(&id).await.unwrap();
    let claimed2 = h.claim_workflow_task("dispatch-queue", "worker-2").await.unwrap();
    assert!(claimed2.is_some(), "should be claimable after release+re-mark");
    assert_eq!(claimed2.unwrap().id, id);
}

// ── Task 3.4 — Events ─────────────────────────────────────────────────────────

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn events_append_and_list(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let wf_id = uid("wf-ev");
    h.create_workflow(&make_workflow(&wf_id, "main", "main")).await.unwrap();

    // Append 3 events
    for seq in 1i32..=3 {
        let ev = make_event(&wf_id, seq);
        let id = h.append_event(&ev).await.unwrap();
        // returned id must be non-negative
        assert!(id >= 0, "append_event should return a non-negative id");
    }

    // list_events returns all 3 in seq order
    let events = h.list_events(&wf_id).await.unwrap();
    assert_eq!(events.len(), 3, "should have 3 events");
    assert_eq!(events[0].seq, 1);
    assert_eq!(events[1].seq, 2);
    assert_eq!(events[2].seq, 3);
    assert_eq!(events[0].workflow_id, wf_id);

    // get_event_count returns 3
    let count = h.get_event_count(&wf_id).await.unwrap();
    assert_eq!(count, 3);

    // Unknown workflow has count 0
    let zero = h.get_event_count("no-such-workflow").await.unwrap();
    assert_eq!(zero, 0);
}

// ── Task 3.5 — Search attributes ──────────────────────────────────────────────

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn search_attrs_upsert(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let wf_id = uid("wf-sa");
    h.create_workflow(&make_workflow(&wf_id, "main", "main")).await.unwrap();

    // First upsert: set two keys
    h.upsert_search_attributes(&wf_id, r#"{"env":"prod","tenant":"acme"}"#)
        .await
        .unwrap();
    let wf = h.get_workflow(&wf_id).await.unwrap().unwrap();
    let sa: serde_json::Value =
        serde_json::from_str(wf.search_attributes.as_deref().unwrap()).unwrap();
    assert_eq!(sa["env"].as_str(), Some("prod"));
    assert_eq!(sa["tenant"].as_str(), Some("acme"));

    // Second upsert: overwrite one key, add another — existing key preserved
    h.upsert_search_attributes(&wf_id, r#"{"env":"staging","region":"eu-west"}"#)
        .await
        .unwrap();
    let wf2 = h.get_workflow(&wf_id).await.unwrap().unwrap();
    let sa2: serde_json::Value =
        serde_json::from_str(wf2.search_attributes.as_deref().unwrap()).unwrap();
    assert_eq!(sa2["env"].as_str(), Some("staging"), "env should be overwritten");
    assert_eq!(sa2["tenant"].as_str(), Some("acme"), "tenant should be preserved");
    assert_eq!(sa2["region"].as_str(), Some("eu-west"), "region should be added");
}

// ── Task 3.3 archival ─────────────────────────────────────────────────────────

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn workflow_archival(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let wf_id = uid("wf-arch");
    h.create_workflow(&make_workflow(&wf_id, "main", "main")).await.unwrap();

    // Complete the workflow so it's eligible for archival
    h.update_workflow_status(&wf_id, assay_domain::types::WorkflowStatus::Completed, None, None)
        .await
        .unwrap();

    // Append an event to be purged
    h.append_event(&make_event(&wf_id, 1)).await.unwrap();

    // list_archivable: cutoff far in the future → should include our workflow
    let far_future = 9_999_999_999.0_f64;
    let archivable = h.list_archivable_workflows(far_future, 10).await.unwrap();
    assert!(archivable.iter().any(|w| w.id == wf_id), "workflow should be archivable");

    // mark_archived_and_purge
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    h.mark_archived_and_purge(&wf_id, "s3://bucket/wf.json", now)
        .await
        .unwrap();

    // Workflow record still exists with archive_uri set
    let wf = h.get_workflow(&wf_id).await.unwrap().unwrap();
    assert!(wf.archived_at.is_some(), "archived_at should be set");
    assert_eq!(wf.archive_uri.as_deref(), Some("s3://bucket/wf.json"));

    // Events purged
    let events = h.list_events(&wf_id).await.unwrap();
    assert!(events.is_empty(), "events should be purged after archival");

    // No longer in archivable list (archived_at is now set)
    let archivable2 = h.list_archivable_workflows(far_future, 10).await.unwrap();
    assert!(!archivable2.iter().any(|w| w.id == wf_id));
}

// ── Helpers for activities / timers / signals ────────────────────────────────

fn make_activity(workflow_id: &str, seq: i32, task_queue: &str) -> assay_domain::types::WorkflowActivity {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    assay_domain::types::WorkflowActivity {
        id: None,
        workflow_id: workflow_id.to_string(),
        seq,
        name: format!("activity-{seq}"),
        task_queue: task_queue.to_string(),
        input: Some(r#"{"n":1}"#.to_string()),
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
        scheduled_at: now,
        started_at: None,
        completed_at: None,
        last_heartbeat: None,
    }
}

fn make_timer(workflow_id: &str, seq: i32, fire_at: f64) -> assay_domain::types::WorkflowTimer {
    assay_domain::types::WorkflowTimer {
        id: None,
        workflow_id: workflow_id.to_string(),
        seq,
        fire_at,
        fired: false,
    }
}

fn make_signal(workflow_id: &str, name: &str) -> assay_domain::types::WorkflowSignal {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    assay_domain::types::WorkflowSignal {
        id: None,
        workflow_id: workflow_id.to_string(),
        name: name.to_string(),
        payload: Some(r#"{"x":1}"#.to_string()),
        consumed: false,
        received_at: now,
    }
}

// ── Task 3.6 — Activities ─────────────────────────────────────────────────────

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn activity_create_and_claim(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let wf_id = uid("wf-act-cc");
    h.create_workflow(&make_workflow(&wf_id, "main", "act-queue")).await.unwrap();

    // Create activity seq=1
    let act = make_activity(&wf_id, 1, "act-queue");
    let id = h.create_activity(&act).await.unwrap();
    assert!(id > 0, "create_activity should return positive id");

    // get_activity
    let got = h.get_activity(id).await.unwrap().expect("should exist");
    assert_eq!(got.workflow_id, wf_id);
    assert_eq!(got.seq, 1);
    assert_eq!(got.status, "PENDING");

    // get_activity_by_workflow_seq
    let got2 = h.get_activity_by_workflow_seq(&wf_id, 1).await.unwrap().expect("should exist by seq");
    assert_eq!(got2.id, Some(id));

    // Two workers race: only one wins.
    let w1 = h.claim_activity("act-queue", "worker-a").await.unwrap();
    let w2 = h.claim_activity("act-queue", "worker-b").await.unwrap();

    assert!(
        (w1.is_some() && w2.is_none()) || (w1.is_none() && w2.is_some()),
        "exactly one worker should win the claim race"
    );
    let winner = w1.or(w2).unwrap();
    assert_eq!(winner.status, "RUNNING");
    assert!(winner.claimed_by.is_some());
}

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn activity_retry_on_failure(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let wf_id = uid("wf-act-rt");
    h.create_workflow(&make_workflow(&wf_id, "main", "retry-q")).await.unwrap();

    let act = make_activity(&wf_id, 1, "retry-q");
    let id = h.create_activity(&act).await.unwrap();

    // Claim and fail the first attempt.
    let claimed = h.claim_activity("retry-q", "worker-x").await.unwrap().expect("should claim");
    assert_eq!(claimed.id, Some(id));
    h.complete_activity(id, None, Some("transient error"), true).await.unwrap();

    let failed = h.get_activity(id).await.unwrap().unwrap();
    assert_eq!(failed.status, "FAILED");

    // Requeue with exponential backoff: attempt=2, next_at = now + 2^1 * 1s
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let next_at = now + 2.0;
    h.requeue_activity_for_retry(id, 2, next_at).await.unwrap();

    let requeued = h.get_activity(id).await.unwrap().unwrap();
    assert_eq!(requeued.status, "PENDING");
    assert_eq!(requeued.attempt, 2);
    assert!(requeued.error.is_none(), "error should be cleared on requeue");
    assert!(requeued.claimed_by.is_none(), "claimed_by should be cleared on requeue");
}

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn activity_heartbeat_timeout(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let wf_id = uid("wf-act-hb");
    h.create_workflow(&make_workflow(&wf_id, "main", "hb-q")).await.unwrap();

    let mut act = make_activity(&wf_id, 1, "hb-q");
    act.heartbeat_timeout_secs = Some(30.0);
    let id = h.create_activity(&act).await.unwrap();

    // Claim it (puts it in RUNNING).
    h.claim_activity("hb-q", "worker-hb").await.unwrap().expect("should claim");

    // Record a heartbeat far in the past (simulate stalled).
    // We do this by calling heartbeat_activity, then checking get_timed_out_activities
    // with a "now" that's 60s after the heartbeat.
    h.heartbeat_activity(id, None).await.unwrap();

    // Verify heartbeat was recorded.
    let a = h.get_activity(id).await.unwrap().unwrap();
    assert!(a.last_heartbeat.is_some(), "last_heartbeat should be set");

    // Simulate "now" = 60s after heartbeat → timeout (threshold 30s).
    let fake_now = a.last_heartbeat.unwrap() + 60.0;
    let timed_out = h.get_timed_out_activities(fake_now).await.unwrap();
    assert!(
        timed_out.iter().any(|a| a.id == Some(id)),
        "stalled activity should be in timed-out list"
    );

    // Before the timeout: no timed-out activities.
    let fresh_now = a.last_heartbeat.unwrap() + 1.0;
    let not_timed_out = h.get_timed_out_activities(fresh_now).await.unwrap();
    assert!(
        !not_timed_out.iter().any(|a| a.id == Some(id)),
        "activity should NOT be in timed-out list when heartbeat is recent"
    );
}

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn activity_cancel_pending(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let wf_id = uid("wf-act-cp");
    h.create_workflow(&make_workflow(&wf_id, "main", "cancel-q")).await.unwrap();

    // Activity 3 is on a separate queue so the claim is deterministic.
    // Activities 1 and 2 stay PENDING on "cancel-q".
    // Activity 3 is on "cancel-claim-q" so exactly it gets claimed.
    let id1 = h.create_activity(&make_activity(&wf_id, 1, "cancel-q")).await.unwrap();
    let id2 = h.create_activity(&make_activity(&wf_id, 2, "cancel-q")).await.unwrap();
    let id3 = h.create_activity(&make_activity(&wf_id, 3, "cancel-claim-q")).await.unwrap();

    // Claim activity 3 (on its own queue) → it becomes RUNNING.
    let claimed = h.claim_activity("cancel-claim-q", "worker-z").await.unwrap();
    assert!(claimed.is_some(), "should claim activity 3");
    assert_eq!(claimed.unwrap().id, Some(id3));

    // Cancel pending activities on this workflow (affects PENDING only).
    let cancelled = h.cancel_pending_activities(&wf_id).await.unwrap();
    assert_eq!(cancelled, 2, "should cancel exactly the 2 PENDING activities");

    let a1 = h.get_activity(id1).await.unwrap().unwrap();
    let a2 = h.get_activity(id2).await.unwrap().unwrap();
    let a3 = h.get_activity(id3).await.unwrap().unwrap();

    assert_eq!(a1.status, "CANCELLED");
    assert_eq!(a2.status, "CANCELLED");
    assert_eq!(a3.status, "RUNNING", "RUNNING activity must not be cancelled");
}

// ── Task 3.7 — Timers ─────────────────────────────────────────────────────────

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn timer_create_and_fire(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let wf_id = uid("wf-tmr-cf");
    h.create_workflow(&make_workflow(&wf_id, "main", "main")).await.unwrap();

    let past = 1_000_000.0_f64; // fire_at in the past
    let timer = make_timer(&wf_id, 1, past);
    let id = h.create_timer(&timer).await.unwrap();
    assert!(id > 0, "create_timer should return positive id");

    // get_timer_by_workflow_seq
    let got = h.get_timer_by_workflow_seq(&wf_id, 1).await.unwrap().expect("should exist");
    assert_eq!(got.id, Some(id));
    assert!(!got.fired, "should not be fired yet");

    // fire_due_timers with "now" > fire_at
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let fired = h.fire_due_timers(now).await.unwrap();
    assert!(
        fired.iter().any(|t| t.id == Some(id)),
        "timer should appear in fire_due_timers result"
    );

    // Now fired=true
    let after = h.get_timer_by_workflow_seq(&wf_id, 1).await.unwrap().unwrap();
    assert!(after.fired, "timer should be marked fired");

    // Second call to fire_due_timers should NOT return already-fired timer.
    let second_fire = h.fire_due_timers(now + 1.0).await.unwrap();
    assert!(
        !second_fire.iter().any(|t| t.id == Some(id)),
        "already-fired timer should not appear again"
    );
}

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn timer_idempotent_on_seq(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let wf_id = uid("wf-tmr-id");
    h.create_workflow(&make_workflow(&wf_id, "main", "main")).await.unwrap();

    let timer = make_timer(&wf_id, 1, 9_999_999_999.0);
    let id1 = h.create_timer(&timer).await.unwrap();
    let id2 = h.create_timer(&timer).await.unwrap(); // same seq → idempotent

    assert_eq!(id1, id2, "creating the same (workflow_id, seq) timer twice should return same id");

    // Only one timer row should exist.
    let got = h.get_timer_by_workflow_seq(&wf_id, 1).await.unwrap().unwrap();
    assert_eq!(got.id, Some(id1));
}

// ── Task 3.8 — Signals ────────────────────────────────────────────────────────

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn signal_send_and_consume(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let wf_id = uid("wf-sig-sc");
    h.create_workflow(&make_workflow(&wf_id, "main", "main")).await.unwrap();

    // Send 2 signals with the same name.
    let sig1 = make_signal(&wf_id, "my-signal");
    let sig2 = make_signal(&wf_id, "my-signal");
    let id1 = h.send_signal(&sig1).await.unwrap();
    let id2 = h.send_signal(&sig2).await.unwrap();
    assert_ne!(id1, id2, "two signals should get distinct ids");

    // consume_signals returns both and marks them consumed.
    let consumed = h.consume_signals(&wf_id, "my-signal").await.unwrap();
    assert_eq!(consumed.len(), 2, "should consume both pending signals");
    assert!(consumed.iter().all(|s| s.consumed), "all returned signals must be marked consumed");

    // Second consume returns nothing.
    let second = h.consume_signals(&wf_id, "my-signal").await.unwrap();
    assert!(second.is_empty(), "second consume should return empty");

    // Different signal name is not consumed.
    let other_sig = make_signal(&wf_id, "other-signal");
    h.send_signal(&other_sig).await.unwrap();
    let other_consumed = h.consume_signals(&wf_id, "other-signal").await.unwrap();
    assert_eq!(other_consumed.len(), 1);
}

// ── Task 3.9 — Schedules ──────────────────────────────────────────────────────

fn make_schedule(namespace: &str, name: &str) -> common::harness::WorkflowSchedule {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    common::harness::WorkflowSchedule {
        namespace: namespace.to_string(),
        name: name.to_string(),
        workflow_type: "cron_wf".to_string(),
        cron_expr: "0 * * * *".to_string(),
        timezone: "UTC".to_string(),
        input: Some(r#"{"key":"val"}"#.to_string()),
        task_queue: "main".to_string(),
        overlap_policy: "skip".to_string(),
        paused: false,
        last_run_at: None,
        next_run_at: None,
        last_workflow_id: None,
        created_at: now,
    }
}

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn schedule_crud(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let sched_name = uid("sched-crud");
    let sched = make_schedule("main", &sched_name);

    // Create
    h.create_schedule(&sched).await.unwrap();

    // Get
    let got = h.get_schedule("main", &sched_name).await.unwrap().expect("should exist");
    assert_eq!(got.name, sched_name);
    assert_eq!(got.cron_expr, "0 * * * *");
    assert!(!got.paused);

    // List
    let list = h.list_schedules("main").await.unwrap();
    assert!(list.iter().any(|s| s.name == sched_name), "should appear in list");

    // Update via patch (cron_expr only)
    let patch = common::harness::SchedulePatch {
        cron_expr: Some("0 0 * * *".to_string()),
        timezone: None,
        input: None,
        task_queue: None,
        overlap_policy: None,
    };
    let updated = h.update_schedule("main", &sched_name, &patch).await.unwrap();
    assert!(updated.is_some(), "update_schedule should return updated record");
    assert_eq!(updated.unwrap().cron_expr, "0 0 * * *");

    // set_paused = true
    let paused = h.set_schedule_paused("main", &sched_name, true).await.unwrap();
    assert!(paused.is_some());
    assert!(paused.unwrap().paused, "should be paused after set_schedule_paused(true)");

    // Verify via get
    let got2 = h.get_schedule("main", &sched_name).await.unwrap().unwrap();
    assert!(got2.paused);

    // set_paused = false
    let resumed = h.set_schedule_paused("main", &sched_name, false).await.unwrap();
    assert!(!resumed.unwrap().paused);

    // Delete
    let deleted = h.delete_schedule("main", &sched_name).await.unwrap();
    assert!(deleted, "delete should return true");

    // Double-delete returns false
    let deleted2 = h.delete_schedule("main", &sched_name).await.unwrap();
    assert!(!deleted2, "second delete should return false");

    // Get after delete returns None
    let gone = h.get_schedule("main", &sched_name).await.unwrap();
    assert!(gone.is_none());
}

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn schedule_last_run_update(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let sched_name = uid("sched-lru");
    h.create_schedule(&make_schedule("main", &sched_name)).await.unwrap();

    let last_run = 1_700_000_000.0_f64;
    let next_run = 1_700_003_600.0_f64;
    let wf_id    = uid("wf-last-run");

    h.update_schedule_last_run("main", &sched_name, last_run, next_run, &wf_id)
        .await
        .unwrap();

    let got = h.get_schedule("main", &sched_name).await.unwrap().unwrap();
    assert_eq!(got.last_run_at, Some(last_run),   "last_run_at should be updated");
    assert_eq!(got.next_run_at, Some(next_run),   "next_run_at should be updated");
    assert_eq!(got.last_workflow_id.as_deref(), Some(wf_id.as_str()),
               "last_workflow_id should be updated");
}

// ── Task 3.10 — Snapshots ─────────────────────────────────────────────────────

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn snapshot_create_and_get_latest(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let wf_id = uid("wf-snap");
    h.create_workflow(&make_workflow(&wf_id, "main", "main")).await.unwrap();

    // No snapshot yet
    let none = h.get_latest_snapshot(&wf_id).await.unwrap();
    assert!(none.is_none(), "no snapshot before any create");

    // Create three snapshots at different event_seqs
    h.create_snapshot(&wf_id, 1, r#"{"step":1}"#).await.unwrap();
    h.create_snapshot(&wf_id, 5, r#"{"step":5}"#).await.unwrap();
    h.create_snapshot(&wf_id, 3, r#"{"step":3}"#).await.unwrap();

    // get_latest should return seq=5 (highest)
    let latest = h.get_latest_snapshot(&wf_id).await.unwrap().expect("should have latest");
    assert_eq!(latest.event_seq, 5, "get_latest should return highest event_seq");
    assert_eq!(latest.state_json, r#"{"step":5}"#);
    assert_eq!(latest.workflow_id, wf_id);

    // Idempotent: re-creating seq=5 with different state updates it
    h.create_snapshot(&wf_id, 5, r#"{"step":5,"updated":true}"#).await.unwrap();
    let updated = h.get_latest_snapshot(&wf_id).await.unwrap().unwrap();
    assert_eq!(updated.event_seq, 5);
    assert_eq!(updated.state_json, r#"{"step":5,"updated":true}"#);
}

// ── Task 3.11 — Archival end-to-end ──────────────────────────────────────────

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn archival_end_to_end(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let wf_id = uid("wf-arch-e2e");
    h.create_workflow(&make_workflow(&wf_id, "main", "main")).await.unwrap();

    // Complete the workflow (sets completed_at to now)
    h.update_workflow_status(&wf_id, assay_domain::types::WorkflowStatus::Completed, None, None)
        .await
        .unwrap();

    // Populate child rows that should be purged
    h.append_event(&make_event(&wf_id, 1)).await.unwrap();
    h.append_event(&make_event(&wf_id, 2)).await.unwrap();
    h.create_activity(&make_activity(&wf_id, 1, "main")).await.unwrap();
    h.create_timer(&make_timer(&wf_id, 1, 9_999_999_999.0)).await.unwrap();
    h.send_signal(&make_signal(&wf_id, "done")).await.unwrap();
    h.create_snapshot(&wf_id, 1, r#"{"s":1}"#).await.unwrap();

    // list_archivable: cutoff far in the future → must include our workflow
    let far_future = 9_999_999_999.0_f64;
    let archivable = h.list_archivable_workflows(far_future, 100).await.unwrap();
    assert!(
        archivable.iter().any(|w| w.id == wf_id),
        "completed workflow should be archivable"
    );

    // Purge
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    h.mark_archived_and_purge(&wf_id, "s3://bucket/e2e.json", now)
        .await
        .unwrap();

    // Workflow record still exists with archive metadata
    let wf = h.get_workflow(&wf_id).await.unwrap().expect("workflow record must remain");
    assert!(wf.archived_at.is_some(), "archived_at should be set");
    assert_eq!(wf.archive_uri.as_deref(), Some("s3://bucket/e2e.json"));

    // All child rows purged
    let events = h.list_events(&wf_id).await.unwrap();
    assert!(events.is_empty(), "events should be purged");

    let snap = h.get_latest_snapshot(&wf_id).await.unwrap();
    assert!(snap.is_none(), "snapshots should be purged");

    // No longer in archivable list
    let archivable2 = h.list_archivable_workflows(far_future, 100).await.unwrap();
    assert!(!archivable2.iter().any(|w| w.id == wf_id),
            "archived workflow must not reappear in archivable list");
}

// ── Task 3.12 — Workers ───────────────────────────────────────────────────────

fn make_worker(id: &str, namespace: &str, task_queue: &str) -> common::harness::WorkflowWorker {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    common::harness::WorkflowWorker {
        id: id.to_string(),
        namespace: namespace.to_string(),
        identity: format!("worker-{id}@host"),
        task_queue: task_queue.to_string(),
        workflows: None,
        activities: None,
        max_concurrent_workflows: 10,
        max_concurrent_activities: 10,
        active_tasks: 0,
        last_heartbeat: now,
        registered_at: now,
    }
}

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn worker_register_heartbeat_list(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");

    let ns = uid("ns-wkr");
    h.create_namespace(&ns).await.unwrap();

    let id1 = uid("wkr-a");
    let id2 = uid("wkr-b");
    let w1 = make_worker(&id1, &ns, "main");
    let w2 = make_worker(&id2, &ns, "main");

    // Register both
    h.register_worker(&w1).await.unwrap();
    h.register_worker(&w2).await.unwrap();

    // list_workers returns both
    let workers = h.list_workers(&ns).await.unwrap();
    assert_eq!(workers.len(), 2, "should have 2 workers");
    let ids: Vec<&str> = workers.iter().map(|w| w.id.as_str()).collect();
    assert!(ids.contains(&id1.as_str()), "w1 should be in list");
    assert!(ids.contains(&id2.as_str()), "w2 should be in list");

    // Heartbeat w1 with a newer timestamp
    let future_hb = w1.last_heartbeat + 100.0;
    h.heartbeat_worker(&id1, future_hb).await.unwrap();

    // Re-list and verify w1 heartbeat updated
    let workers2 = h.list_workers(&ns).await.unwrap();
    let w1_after = workers2.iter().find(|w| w.id == id1).unwrap();
    assert!(
        (w1_after.last_heartbeat - future_hb).abs() < 1.0,
        "heartbeat should be updated for w1"
    );

    // Idempotent register: re-registering w1 should not error
    h.register_worker(&w1).await.unwrap();
    let workers3 = h.list_workers(&ns).await.unwrap();
    assert_eq!(workers3.len(), 2, "re-register should not duplicate");
}

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn worker_remove_dead(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");

    let ns = uid("ns-dead");
    h.create_namespace(&ns).await.unwrap();

    // t=0: register both workers with heartbeat at epoch 1000
    let base_time = 1_000.0_f64;
    let id_live = uid("wkr-live");
    let id_dead = uid("wkr-dead");

    let mut w_live = make_worker(&id_live, &ns, "main");
    w_live.last_heartbeat = base_time;
    w_live.registered_at  = base_time;

    let mut w_dead = make_worker(&id_dead, &ns, "main");
    w_dead.last_heartbeat = base_time;
    w_dead.registered_at  = base_time;

    h.register_worker(&w_live).await.unwrap();
    h.register_worker(&w_dead).await.unwrap();

    // Advance live worker's heartbeat to base+200
    h.heartbeat_worker(&id_live, base_time + 200.0).await.unwrap();
    // Dead worker stays at base_time

    // remove_dead_workers with cutoff = base+100:
    // dead worker (heartbeat=base_time < base+100) should be removed
    // live worker (heartbeat=base+200 >= base+100) should survive
    let cutoff = base_time + 100.0;
    let removed = h.remove_dead_workers(cutoff).await.unwrap();
    assert_eq!(removed.len(), 1, "exactly one dead worker should be removed");
    assert_eq!(removed[0], id_dead, "the removed id should be the dead worker");

    // list_workers should only contain the live worker
    let remaining = h.list_workers(&ns).await.unwrap();
    assert_eq!(remaining.len(), 1, "only live worker should remain");
    assert_eq!(remaining[0].id, id_live);
}

// ── Task 3.14 — Queue Stats ───────────────────────────────────────────────────

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn queue_stats(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let ns = uid("ns-qs");
    h.create_namespace(&ns).await.unwrap();

    // Create two workflows on different task queues
    let wf_id1 = uid("wf-qs1");
    let wf_id2 = uid("wf-qs2");
    let mut wf1 = make_workflow(&wf_id1, &ns, "queue-alpha");
    wf1.task_queue = "queue-alpha".to_string();
    let mut wf2 = make_workflow(&wf_id2, &ns, "queue-beta");
    wf2.task_queue = "queue-beta".to_string();
    h.create_workflow(&wf1).await.unwrap();
    h.create_workflow(&wf2).await.unwrap();

    // Add activities: 2 PENDING on alpha, 1 PENDING on beta
    let a1 = make_activity(&wf_id1, 1, "queue-alpha");
    let a2 = make_activity(&wf_id1, 2, "queue-alpha");
    let a3 = make_activity(&wf_id2, 1, "queue-beta");
    h.create_activity(&a1).await.unwrap();
    h.create_activity(&a2).await.unwrap();
    h.create_activity(&a3).await.unwrap();

    // Register workers: 2 on alpha, 1 on beta
    let w1 = make_worker(&uid("qs-w1"), &ns, "queue-alpha");
    let w2 = make_worker(&uid("qs-w2"), &ns, "queue-alpha");
    let w3 = make_worker(&uid("qs-w3"), &ns, "queue-beta");
    h.register_worker(&w1).await.unwrap();
    h.register_worker(&w2).await.unwrap();
    h.register_worker(&w3).await.unwrap();

    // Get queue stats
    let stats = h.get_queue_stats(&ns).await.unwrap();

    // Should have entries for both queues
    let alpha = stats.iter().find(|s| s.queue == "queue-alpha");
    let beta  = stats.iter().find(|s| s.queue == "queue-beta");

    assert!(alpha.is_some(), "queue-alpha should appear in stats");
    assert!(beta.is_some(),  "queue-beta should appear in stats");

    let alpha = alpha.unwrap();
    let beta  = beta.unwrap();

    assert_eq!(alpha.pending_activities, 2, "queue-alpha should have 2 pending activities");
    assert_eq!(alpha.running_activities, 0, "queue-alpha should have 0 running activities");
    assert_eq!(alpha.workers, 2, "queue-alpha should have 2 workers");

    assert_eq!(beta.pending_activities, 1, "queue-beta should have 1 pending activity");
    assert_eq!(beta.running_activities, 0, "queue-beta should have 0 running activities");
    assert_eq!(beta.workers, 1, "queue-beta should have 1 worker");
}

// ── Task 3.14 — Child Workflows ───────────────────────────────────────────────

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn child_workflows_listing(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");

    // Create parent workflow
    let parent_id = uid("wf-parent");
    h.create_workflow(&make_workflow(&parent_id, "main", "main"))
        .await
        .unwrap();

    // Create 3 child workflows with parent_id set
    let child_ids: Vec<String> = (1..=3).map(|i| uid(&format!("wf-child{i}"))).collect();
    for child_id in &child_ids {
        let mut child = make_workflow(child_id, "main", "main");
        child.parent_id = Some(parent_id.clone());
        h.create_workflow(&child).await.unwrap();
    }

    // Create an unrelated workflow (no parent_id)
    let unrelated = uid("wf-unrelated");
    h.create_workflow(&make_workflow(&unrelated, "main", "main"))
        .await
        .unwrap();

    // list_child_workflows returns exactly the 3 children
    let children = h.list_child_workflows(&parent_id).await.unwrap();
    assert_eq!(children.len(), 3, "should return exactly 3 children");
    for child_id in &child_ids {
        assert!(
            children.iter().any(|c| &c.id == child_id),
            "child {child_id} should appear in list"
        );
    }
    // None of the children should be the unrelated workflow
    assert!(
        !children.iter().any(|c| c.id == unrelated),
        "unrelated workflow must not appear in child list"
    );

    // Querying by a non-existent parent returns empty
    let none = h.list_child_workflows("no-such-parent").await.unwrap();
    assert!(none.is_empty(), "list_child_workflows with unknown parent should return empty");
}

// ── Task 3.15 — Leader Election ───────────────────────────────────────────────

#[rstest]
#[cfg_attr(all(feature = "backend-postgres", target_os = "linux"), case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]

#[tokio::test(flavor = "multi_thread")]
async fn leader_election(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");

    // First acquire from this store instance should always succeed.
    let first = h.try_acquire_scheduler_lock().await.unwrap();
    assert!(first, "first try_acquire_scheduler_lock should return true");

    // Second call from the same store instance.
    //
    // Backend semantics differ:
    //
    // - SQLite:   always returns true (single-process leader).
    // - Postgres: pg_try_advisory_lock is session-scoped. A connection pool
    //             may route the second call to a different session, returning
    //             false. We don't assert the second result for PG — the
    //             contract only requires the first call to succeed and that
    //             a concurrent *different* process would get false.
    //
    // For SQLite we DO assert true on the second call.
    let second = h.try_acquire_scheduler_lock().await.unwrap();
    #[cfg(feature = "backend-sqlite")]
    if matches!(h, common::harness::Harness::Sqlite { .. }) {
        assert!(second, "SQLite: second acquire should be true (always leader)");
    }
    // For Postgres: second call may be true or false depending on pool routing.
    // We just verify it doesn't error.
    let _ = second;
}

// ── Task 3.16 / 3.17 — Push streams ──────────────────────────────────────────
//
// SQLite is intentionally NOT tested here: it returns stream::empty() by
// design (no cross-process notification primitive — hybrid model,
// § "Dispatch wake-up").
//
// If the 5-second timeout proves flaky on a backend (e.g. slow CI), do NOT
// increase it — flaky push tests hide real bugs. Instead investigate the
// timing between subscription setup and the triggering insert.

// Push-stream tests (push_runnable_fires_on_dispatchable +
// push_tasks_fires_on_activity_insert) have been removed in v0.13.1.
// The PL/pgSQL triggers + WorkflowStore::subscribe_runnable/_tasks they
// exercised are gone; the engine-events outbox
// (`assay_domain::events::{PgEngineEventBus, SqliteEngineEventBus}`)
// replaces them. Equivalent coverage lives in that crate's pg_test +
// sqlite_test suites (append_then_read_round_trip, subscribe_receives_*,
// etc.).
