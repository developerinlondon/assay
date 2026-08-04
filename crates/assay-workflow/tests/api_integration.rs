use assay_workflow::{SqliteStore, WorkflowCtx};
use std::sync::Arc;

/// Helper: start engine + API on a random port, return the base URL.
async fn start_test_server() -> (String, tokio::task::JoinHandle<()>) {
    let store = SqliteStore::new("sqlite::memory:").await.unwrap();
    let state = Arc::new(WorkflowCtx::start(Arc::new(store)));

    let app = assay_workflow::api::router(state, |r| r);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{port}");

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give the server a moment to start
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    (base_url, handle)
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

async fn prepare_failed_activity_workflow(
    state: &Arc<WorkflowCtx<SqliteStore>>,
    workflow_id: &str,
) -> (i64, i64) {
    state
        .start_workflow(
            "main",
            "DeploymentWorkflow",
            workflow_id,
            None,
            "deployments",
            None,
        )
        .await
        .unwrap();
    let failed = state
        .schedule_activity(
            workflow_id,
            1,
            "update_config",
            None,
            "deployments",
            assay_workflow::types::ScheduleActivityOpts {
                max_attempts: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let failed_id = failed.id.unwrap();
    state
        .claim_activity("deployments", "worker-1")
        .await
        .unwrap()
        .expect("failed activity should be claimable");
    state
        .fail_activity(failed_id, "dependency rejected the update")
        .await
        .unwrap();
    let notification = state
        .schedule_activity(
            workflow_id,
            2,
            "notify_failure",
            None,
            "deployments",
            Default::default(),
        )
        .await
        .unwrap();
    let notification_id = notification.id.unwrap();
    state
        .claim_activity("deployments", "worker-1")
        .await
        .unwrap()
        .expect("notification activity should be claimable");
    state
        .complete_activity(notification_id, Some(r#"{"sent":true}"#), None, false)
        .await
        .unwrap();
    state
        .fail_workflow(workflow_id, "activity 'update_config' failed")
        .await
        .unwrap();
    (failed_id, notification_id)
}

#[tokio::test]
async fn get_events_keeps_the_public_two_argument_handler_contract() {
    let store = SqliteStore::new("sqlite::memory:").await.unwrap();
    let state = Arc::new(WorkflowCtx::start(Arc::new(store)));

    let result = assay_workflow::api::workflows::get_events(
        axum::extract::State(state),
        axum::extract::Path("missing-workflow".to_string()),
    )
    .await;
    let response = match result {
        Ok(response) => response,
        Err(_) => panic!("legacy get_events handler failed"),
    };

    assert!(response.0.is_empty());
}

#[tokio::test]
async fn explicit_ascending_order_uses_default_bounded_page() {
    let (url, _handle) = start_test_server().await;
    let c = client();

    let response = c
        .post(format!("{url}/api/v1/engine/workflow/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "PagedHistory",
            "workflow_id": "wf-ascending-page",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);

    for index in 1..=50 {
        let response = c
            .post(format!(
                "{url}/api/v1/engine/workflow/workflows/wf-ascending-page/signal/page-{index}"
            ))
            .json(&serde_json::json!({ "payload": { "index": index } }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }

    let response = c
        .get(format!(
            "{url}/api/v1/engine/workflow/workflows/wf-ascending-page/events?order=asc"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let page: Vec<serde_json::Value> = response.json().await.unwrap();
    assert_eq!(page.len(), 50);
    assert_eq!(page.first().unwrap()["seq"], 1);
    assert_eq!(page.last().unwrap()["seq"], 50);
}

#[tokio::test]
async fn health_check() {
    let (url, _handle) = start_test_server().await;

    let resp = client()
        .get(format!("{url}/api/v1/engine/workflow/health"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["service"], "assay-workflow");
}

#[tokio::test]
async fn start_and_list_workflows() {
    let (url, _handle) = start_test_server().await;
    let c = client();

    // Start a workflow
    let resp = c
        .post(format!("{url}/api/v1/engine/workflow/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "IngestData",
            "workflow_id": "wf-test-1",
            "input": {"source": "s3://bucket"},
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["workflow_id"], "wf-test-1");
    assert_eq!(body["status"], "PENDING");

    // List workflows
    let resp = c
        .get(format!("{url}/api/v1/engine/workflow/workflows"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(body.len(), 1);
    assert_eq!(body[0]["id"], "wf-test-1");

    // Describe workflow
    let resp = c
        .get(format!("{url}/api/v1/engine/workflow/workflows/wf-test-1"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["workflow_type"], "IngestData");

    // Get events
    let resp = c
        .get(format!(
            "{url}/api/v1/engine/workflow/workflows/wf-test-1/events"
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(body.len(), 1);
    assert_eq!(body[0]["event_type"], "WorkflowStarted");

    for index in 1..=3 {
        let response = c
            .post(format!(
                "{url}/api/v1/engine/workflow/workflows/wf-test-1/signal/page-{index}"
            ))
            .json(&serde_json::json!({ "payload": { "index": index } }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }

    let response = c
        .get(format!(
            "{url}/api/v1/engine/workflow/workflows/wf-test-1/events?limit=2&order=desc"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let page: Vec<serde_json::Value> = response.json().await.unwrap();
    assert_eq!(page.len(), 2);
    assert_eq!(page[0]["seq"], 4);
    assert_eq!(page[1]["seq"], 3);

    let response = c
        .get(format!(
            "{url}/api/v1/engine/workflow/workflows/wf-test-1/events?limit=2&order=desc&cursor=3"
        ))
        .send()
        .await
        .unwrap();
    let page: Vec<serde_json::Value> = response.json().await.unwrap();
    assert_eq!(
        page.iter()
            .map(|event| event["seq"].as_i64())
            .collect::<Vec<_>>(),
        [Some(2), Some(1)]
    );
}

#[tokio::test]
async fn signal_and_cancel_workflow() {
    let (url, _handle) = start_test_server().await;
    let c = client();

    // Start
    c.post(format!("{url}/api/v1/engine/workflow/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "Approval",
            "workflow_id": "wf-sig-1",
        }))
        .send()
        .await
        .unwrap();

    // Send signal
    let resp = c
        .post(format!(
            "{url}/api/v1/engine/workflow/workflows/wf-sig-1/signal/approve"
        ))
        .json(&serde_json::json!({ "payload": {"approved": true} }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Cancel
    let resp = c
        .post(format!(
            "{url}/api/v1/engine/workflow/workflows/wf-sig-1/cancel"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Cancel again — should 404 (already terminal)
    let resp = c
        .post(format!(
            "{url}/api/v1/engine/workflow/workflows/wf-sig-1/cancel"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

// Regression for issue #66: stdlib used to send `[]` (empty Lua table → JSON
// array) for no-body POSTs, which the Option<Json<CancelBody>> extractor
// rejected with 400. The handler now consumes raw bytes and tolerates any
// of: missing body, "{}", "[]", '{"reason":"..."}'.
#[tokio::test]
async fn cancel_accepts_any_body_shape() {
    let (url, _handle) = start_test_server().await;
    let c = client();

    for (i, body_kind) in ["none", "empty_object", "empty_array", "with_reason"]
        .iter()
        .enumerate()
    {
        let wf_id = format!("wf-cancel-{i}");
        c.post(format!("{url}/api/v1/engine/workflow/workflows"))
            .json(&serde_json::json!({
                "workflow_type": "Approval",
                "workflow_id": wf_id,
            }))
            .send()
            .await
            .unwrap();

        let cancel_url = format!("{url}/api/v1/engine/workflow/workflows/{wf_id}/cancel");
        let req = c.post(&cancel_url);
        let req = match *body_kind {
            "none" => req,
            "empty_object" => req.header("content-type", "application/json").body("{}"),
            "empty_array" => req.header("content-type", "application/json").body("[]"),
            "with_reason" => req
                .header("content-type", "application/json")
                .body(r#"{"reason":"explicit"}"#),
            _ => unreachable!(),
        };
        let resp = req.send().await.unwrap();
        assert_eq!(
            resp.status(),
            200,
            "cancel with body_kind={body_kind} should be 200"
        );
    }
}

#[tokio::test]
async fn worker_register_and_poll() {
    let (url, _handle) = start_test_server().await;
    let c = client();

    // Register worker
    let resp = c
        .post(format!("{url}/api/v1/engine/workflow/workers/register"))
        .json(&serde_json::json!({
            "identity": "test-worker-1",
            "queue": "default",
            "activities": ["fetch_data"],
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let worker_id = body["worker_id"].as_str().unwrap().to_string();
    assert!(worker_id.starts_with("w-"));

    // List workers
    let resp = c
        .get(format!("{url}/api/v1/engine/workflow/workers"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(body.len(), 1);

    // Poll for task (none available)
    let resp = c
        .post(format!("{url}/api/v1/engine/workflow/tasks/poll"))
        .json(&serde_json::json!({
            "queue": "default",
            "worker_id": worker_id,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["task"].is_null());
}

#[tokio::test]
async fn schedule_crud() {
    let (url, _handle) = start_test_server().await;
    let c = client();

    // Create schedule
    let resp = c
        .post(format!("{url}/api/v1/engine/workflow/schedules"))
        .json(&serde_json::json!({
            "name": "hourly-ingest",
            "workflow_type": "IngestData",
            "cron_expr": "0 * * * *",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    // List schedules
    let resp = c
        .get(format!("{url}/api/v1/engine/workflow/schedules"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(body.len(), 1);
    assert_eq!(body[0]["name"], "hourly-ingest");

    // Get schedule
    let resp = c
        .get(format!(
            "{url}/api/v1/engine/workflow/schedules/hourly-ingest"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Delete schedule
    let resp = c
        .delete(format!(
            "{url}/api/v1/engine/workflow/schedules/hourly-ingest"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Verify deleted
    let resp = c
        .get(format!(
            "{url}/api/v1/engine/workflow/schedules/hourly-ingest"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn workflow_not_found() {
    let (url, _handle) = start_test_server().await;
    let c = client();

    let resp = c
        .get(format!(
            "{url}/api/v1/engine/workflow/workflows/nonexistent"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn schedule_patch_updates_fields() {
    let (url, _h) = start_test_server().await;
    let c = client();

    // Create
    let resp = c
        .post(format!("{url}/api/v1/engine/workflow/schedules"))
        .json(&serde_json::json!({
            "name": "nightly",
            "workflow_type": "Report",
            "cron_expr": "0 0 2 * * *",
            "timezone": "UTC",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "create schedule");

    // Patch cron + timezone + input
    let resp = c
        .patch(format!("{url}/api/v1/engine/workflow/schedules/nightly"))
        .json(&serde_json::json!({
            "cron_expr": "0 0 3 * * *",
            "timezone": "Europe/Berlin",
            "input": { "lookback_hours": 24 },
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "patch schedule");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["cron_expr"], "0 0 3 * * *");
    assert_eq!(body["timezone"], "Europe/Berlin");
    let input_str = body["input"].as_str().expect("input string");
    let input: serde_json::Value = serde_json::from_str(input_str).unwrap();
    assert_eq!(input["lookback_hours"], 24);

    // Patch with unchanged fields preserves them
    let resp = c
        .patch(format!("{url}/api/v1/engine/workflow/schedules/nightly"))
        .json(&serde_json::json!({ "task_queue": "reports" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["task_queue"], "reports");
    assert_eq!(
        body["cron_expr"], "0 0 3 * * *",
        "cron kept from prior patch"
    );
    assert_eq!(
        body["timezone"], "Europe/Berlin",
        "timezone kept from prior patch"
    );
}

#[tokio::test]
async fn schedule_pause_and_resume() {
    let (url, _h) = start_test_server().await;
    let c = client();

    c.post(format!("{url}/api/v1/engine/workflow/schedules"))
        .json(&serde_json::json!({
            "name": "hourly",
            "workflow_type": "Report",
            "cron_expr": "0 0 * * * *",
        }))
        .send()
        .await
        .unwrap();

    // Pause
    let resp = c
        .post(format!(
            "{url}/api/v1/engine/workflow/schedules/hourly/pause"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["paused"], true);

    // Resume
    let resp = c
        .post(format!(
            "{url}/api/v1/engine/workflow/schedules/hourly/resume"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["paused"], false);
}

#[tokio::test]
async fn schedule_patch_404_on_missing() {
    let (url, _h) = start_test_server().await;
    let c = client();
    let resp = c
        .patch(format!("{url}/api/v1/engine/workflow/schedules/ghost"))
        .json(&serde_json::json!({ "cron_expr": "0 0 * * * *" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn schedule_patch_rejects_invalid_timezone() {
    let (url, _h) = start_test_server().await;
    let c = client();
    c.post(format!("{url}/api/v1/engine/workflow/schedules"))
        .json(&serde_json::json!({
            "name": "x",
            "workflow_type": "T",
            "cron_expr": "0 0 * * * *",
        }))
        .send()
        .await
        .unwrap();
    let resp = c
        .patch(format!("{url}/api/v1/engine/workflow/schedules/x"))
        .json(&serde_json::json!({ "timezone": "Not/AZone" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 500, "invalid timezone rejected");
}

#[tokio::test]
async fn version_endpoint_returns_shape() {
    let (url, _h) = start_test_server().await;
    let c = client();
    let resp = c
        .get(format!("{url}/api/v1/engine/workflow/version"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["version"].is_string(), "version is a string");
    let profile = body["build_profile"]
        .as_str()
        .expect("build_profile string");
    assert!(
        profile == "debug" || profile == "release",
        "build_profile one of debug|release, got {profile}"
    );
}

#[tokio::test]
async fn retry_failed_activity_resumes_at_the_failure_boundary() {
    let store = SqliteStore::new("sqlite::memory:").await.unwrap();
    let state = Arc::new(WorkflowCtx::start(Arc::new(store)));
    let (failed_id, notification_id) =
        prepare_failed_activity_workflow(&state, "wf-retry-failed").await;

    let app = assay_workflow::api::router(Arc::clone(&state), |router| router);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let response = client()
        .post(format!(
            "http://127.0.0.1:{port}/api/v1/engine/workflow/workflows/wf-retry-failed/retry"
        ))
        .json(&serde_json::json!({
            "requested_by": "operator@example.com",
            "reason": "dependency configuration corrected",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["workflow_id"], "wf-retry-failed");
    assert_eq!(body["status"], "WAITING");
    assert_eq!(body["activity"]["id"], failed_id);
    assert_eq!(body["activity"]["seq"], 1);
    assert_eq!(body["activity"]["name"], "update_config");
    assert_eq!(body["activity"]["status"], "PENDING");
    assert_eq!(body["activity"]["attempt"], 1);
    assert_eq!(body["invalidated_activities"], 1);

    let workflow = state
        .get_workflow("wf-retry-failed")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(workflow.status, "WAITING");
    assert!(workflow.error.is_none());
    assert!(workflow.completed_at.is_none());
    assert!(state.get_activity(notification_id).await.unwrap().is_none());

    let events = state.get_events("wf-retry-failed").await.unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "ActivityFailed")
    );
    assert!(events.iter().any(|event| {
        event.event_type == "ActivityCompleted"
            && event
                .payload
                .as_deref()
                .is_some_and(|payload| payload.contains("notify_failure"))
    }));
    let retry = events
        .iter()
        .find(|event| event.event_type == "ActivityRetryRequested")
        .expect("retry request should be retained in workflow history");
    let retry_payload: serde_json::Value =
        serde_json::from_str(retry.payload.as_deref().unwrap()).unwrap();
    assert_eq!(retry_payload["activity_seq"], 1);
    assert_eq!(retry_payload["requested_by"], "operator@example.com");
    assert_eq!(
        retry_payload["reason"],
        "dependency configuration corrected"
    );

    handle.abort();
}

#[tokio::test]
async fn concurrent_retry_requests_schedule_the_failed_activity_once() {
    let store = SqliteStore::new("sqlite::memory:").await.unwrap();
    let state = Arc::new(WorkflowCtx::start(Arc::new(store)));
    prepare_failed_activity_workflow(&state, "wf-concurrent-retry").await;
    let app = assay_workflow::api::router(Arc::clone(&state), |router| router);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let retry_url = format!(
        "http://127.0.0.1:{port}/api/v1/engine/workflow/workflows/wf-concurrent-retry/retry"
    );
    let first_client = client();
    let second_client = client();
    let first = first_client.post(&retry_url).json(&serde_json::json!({
        "requested_by": "operator-a@example.com",
        "reason": "dependency corrected",
    }));
    let second = second_client.post(&retry_url).json(&serde_json::json!({
        "requested_by": "operator-b@example.com",
        "reason": "dependency corrected",
    }));
    let (first_response, second_response) = tokio::join!(first.send(), second.send());
    let mut statuses = [
        first_response.unwrap().status().as_u16(),
        second_response.unwrap().status().as_u16(),
    ];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 409]);

    let events = state.get_events("wf-concurrent-retry").await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "ActivityRetryRequested")
            .count(),
        1
    );
    let claimed = state
        .claim_activity("deployments", "worker-retry")
        .await
        .unwrap()
        .expect("one retry should be scheduled");
    assert_eq!(claimed.seq, 1);
    assert!(
        state
            .claim_activity("deployments", "worker-retry")
            .await
            .unwrap()
            .is_none(),
        "a concurrent request must not create a second activity"
    );
    handle.abort();
}
