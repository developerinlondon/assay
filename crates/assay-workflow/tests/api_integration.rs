use assay_workflow::{Engine, SqliteStore};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Helper: start engine + API on a random port, return the base URL.
async fn start_test_server() -> (String, tokio::task::JoinHandle<()>) {
    let store = SqliteStore::new("sqlite::memory:").await.unwrap();
    let engine = Engine::start(store);

    let (event_tx, _) = broadcast::channel(64);
    let state = Arc::new(assay_workflow::api::AppState {
        engine: Arc::new(engine),
        event_tx,
        auth_mode: assay_workflow::api::auth::AuthMode::no_auth(),
        binary_version: None,
    });

    let app = assay_workflow::api::router(state);
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

#[tokio::test]
async fn health_check() {
    let (url, _handle) = start_test_server().await;

    let resp = client()
        .get(format!("{url}/api/v1/health"))
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
        .post(format!("{url}/api/v1/workflows"))
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
        .get(format!("{url}/api/v1/workflows"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(body.len(), 1);
    assert_eq!(body[0]["id"], "wf-test-1");

    // Describe workflow
    let resp = c
        .get(format!("{url}/api/v1/workflows/wf-test-1"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["workflow_type"], "IngestData");

    // Get events
    let resp = c
        .get(format!("{url}/api/v1/workflows/wf-test-1/events"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(body.len(), 1);
    assert_eq!(body[0]["event_type"], "WorkflowStarted");
}

#[tokio::test]
async fn signal_and_cancel_workflow() {
    let (url, _handle) = start_test_server().await;
    let c = client();

    // Start
    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "Approval",
            "workflow_id": "wf-sig-1",
        }))
        .send()
        .await
        .unwrap();

    // Send signal
    let resp = c
        .post(format!("{url}/api/v1/workflows/wf-sig-1/signal/approve"))
        .json(&serde_json::json!({ "payload": {"approved": true} }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Cancel
    let resp = c
        .post(format!("{url}/api/v1/workflows/wf-sig-1/cancel"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Cancel again — should 404 (already terminal)
    let resp = c
        .post(format!("{url}/api/v1/workflows/wf-sig-1/cancel"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn worker_register_and_poll() {
    let (url, _handle) = start_test_server().await;
    let c = client();

    // Register worker
    let resp = c
        .post(format!("{url}/api/v1/workers/register"))
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
        .get(format!("{url}/api/v1/workers"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(body.len(), 1);

    // Poll for task (none available)
    let resp = c
        .post(format!("{url}/api/v1/tasks/poll"))
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
        .post(format!("{url}/api/v1/schedules"))
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
        .get(format!("{url}/api/v1/schedules"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(body.len(), 1);
    assert_eq!(body[0]["name"], "hourly-ingest");

    // Get schedule
    let resp = c
        .get(format!("{url}/api/v1/schedules/hourly-ingest"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Delete schedule
    let resp = c
        .delete(format!("{url}/api/v1/schedules/hourly-ingest"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Verify deleted
    let resp = c
        .get(format!("{url}/api/v1/schedules/hourly-ingest"))
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
        .get(format!("{url}/api/v1/workflows/nonexistent"))
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
        .post(format!("{url}/api/v1/schedules"))
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
        .patch(format!("{url}/api/v1/schedules/nightly"))
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
        .patch(format!("{url}/api/v1/schedules/nightly"))
        .json(&serde_json::json!({ "task_queue": "reports" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["task_queue"], "reports");
    assert_eq!(body["cron_expr"], "0 0 3 * * *", "cron kept from prior patch");
    assert_eq!(body["timezone"], "Europe/Berlin", "timezone kept from prior patch");
}

#[tokio::test]
async fn schedule_pause_and_resume() {
    let (url, _h) = start_test_server().await;
    let c = client();

    c.post(format!("{url}/api/v1/schedules"))
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
        .post(format!("{url}/api/v1/schedules/hourly/pause"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["paused"], true);

    // Resume
    let resp = c
        .post(format!("{url}/api/v1/schedules/hourly/resume"))
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
        .patch(format!("{url}/api/v1/schedules/ghost"))
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
    c.post(format!("{url}/api/v1/schedules"))
        .json(&serde_json::json!({
            "name": "x",
            "workflow_type": "T",
            "cron_expr": "0 0 * * * *",
        }))
        .send()
        .await
        .unwrap();
    let resp = c
        .patch(format!("{url}/api/v1/schedules/x"))
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
    let resp = c.get(format!("{url}/api/v1/version")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["version"].is_string(), "version is a string");
    let profile = body["build_profile"].as_str().expect("build_profile string");
    assert!(
        profile == "debug" || profile == "release",
        "build_profile one of debug|release, got {profile}"
    );
}
