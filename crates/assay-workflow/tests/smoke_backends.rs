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
#[cfg_attr(feature = "backend-postgres", case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]
#[cfg_attr(feature = "backend-surrealdb", case::surreal(Backend::Surreal))]
#[tokio::test(flavor = "multi_thread")]
async fn namespace_roundtrip(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    // "main" namespace is created during harness setup — it must appear in the list.
    let list = h.list_namespaces().await.unwrap();
    assert!(list.iter().any(|n| n.name == "main"));
}

// ── Task 3.2 — Namespaces ─────────────────────────────────────────────────────

#[rstest]
#[cfg_attr(feature = "backend-postgres", case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]
#[cfg_attr(feature = "backend-surrealdb", case::surreal(Backend::Surreal))]
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
#[cfg_attr(feature = "backend-postgres", case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]
#[cfg_attr(feature = "backend-surrealdb", case::surreal(Backend::Surreal))]
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
#[cfg_attr(feature = "backend-postgres", case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]
#[cfg_attr(feature = "backend-surrealdb", case::surreal(Backend::Surreal))]
#[tokio::test(flavor = "multi_thread")]
async fn workflow_status_transitions(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let id = uid("wf-st");
    h.create_workflow(&make_workflow(&id, "main", "main")).await.unwrap();

    // PENDING → RUNNING
    h.update_workflow_status(&id, assay_core::types::WorkflowStatus::Running, None, None)
        .await
        .unwrap();
    let wf = h.get_workflow(&id).await.unwrap().unwrap();
    assert_eq!(wf.status, "RUNNING");
    assert!(wf.completed_at.is_none(), "completed_at should be None while running");

    // RUNNING → COMPLETED (terminal)
    h.update_workflow_status(
        &id,
        assay_core::types::WorkflowStatus::Completed,
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
#[cfg_attr(feature = "backend-postgres", case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]
#[cfg_attr(feature = "backend-surrealdb", case::surreal(Backend::Surreal))]
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
#[cfg_attr(feature = "backend-postgres", case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]
#[cfg_attr(feature = "backend-surrealdb", case::surreal(Backend::Surreal))]
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
        .list_workflows(&ns, Some(assay_core::types::WorkflowStatus::Pending), None, None, 100, 0)
        .await
        .unwrap();
    assert_eq!(pending.len(), 2);

    // Update one to RUNNING, then filter
    h.update_workflow_status(&id1, assay_core::types::WorkflowStatus::Running, None, None)
        .await
        .unwrap();
    let running = h
        .list_workflows(&ns, Some(assay_core::types::WorkflowStatus::Running), None, None, 100, 0)
        .await
        .unwrap();
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].id, id1);
}

#[rstest]
#[cfg_attr(feature = "backend-postgres", case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]
#[cfg_attr(feature = "backend-surrealdb", case::surreal(Backend::Surreal))]
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
#[cfg_attr(feature = "backend-postgres", case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]
#[cfg_attr(feature = "backend-surrealdb", case::surreal(Backend::Surreal))]
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
#[cfg_attr(feature = "backend-postgres", case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]
#[cfg_attr(feature = "backend-surrealdb", case::surreal(Backend::Surreal))]
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
#[cfg_attr(feature = "backend-postgres", case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]
#[cfg_attr(feature = "backend-surrealdb", case::surreal(Backend::Surreal))]
#[tokio::test(flavor = "multi_thread")]
async fn workflow_archival(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    let wf_id = uid("wf-arch");
    h.create_workflow(&make_workflow(&wf_id, "main", "main")).await.unwrap();

    // Complete the workflow so it's eligible for archival
    h.update_workflow_status(&wf_id, assay_core::types::WorkflowStatus::Completed, None, None)
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
