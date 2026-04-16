//! Phase 9 — orchestration runtime tests.
//!
//! These tests verify that the workflow engine actually executes workflows
//! end-to-end: activities get scheduled, completed, retried, and the workflow
//! progresses to a terminal state. They are the acceptance contract for
//! Phase 9 in `.claude/plans/03-assay-11-workflow-runtime.md`.
//!
//! Each test starts a real engine (in-memory SQLite), exercises the REST
//! surface, and asserts on persistent state — never on logs or stdout.

use assay_workflow::{Engine, SqliteStore};
use std::sync::Arc;
use tokio::sync::broadcast;

async fn start_test_server() -> (String, tokio::task::JoinHandle<()>) {
    let store = SqliteStore::new("sqlite::memory:").await.unwrap();
    let engine = Engine::start(store);

    let (event_tx, _) = broadcast::channel(64);
    let state = Arc::new(assay_workflow::api::AppState {
        engine: Arc::new(engine),
        event_tx,
        auth_mode: assay_workflow::api::auth::AuthMode::NoAuth,
    });

    let app = assay_workflow::api::router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{port}");

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (base_url, handle)
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

/// 9.1.5 — Activity scheduling endpoint:
///   - POST /workflows starts a workflow
///   - POST /workflows/:id/activities schedules an activity
///   - GET  /activities/:id returns the activity record
///   - The workflow event log contains WorkflowStarted + ActivityScheduled
#[tokio::test]
async fn schedule_activity_creates_pending_row_and_event() {
    let (url, _h) = start_test_server().await;
    let c = client();

    // 1. Start workflow
    let resp = c
        .post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "TestWorkflow",
            "workflow_id": "wf-1",
            "task_queue": "default",
            "input": {"hello": "world"},
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "start workflow");

    // 2. Schedule activity at seq=1
    let resp = c
        .post(format!("{url}/api/v1/workflows/wf-1/activities"))
        .json(&serde_json::json!({
            "name": "fetch",
            "input": {"url": "https://example.com"},
            "task_queue": "default",
            "seq": 1,
            "max_attempts": 3,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "schedule activity");
    let scheduled: serde_json::Value = resp.json().await.unwrap();
    let activity_id = scheduled["id"].as_i64().expect("activity id");

    // 3. GET activity returns it with status PENDING
    let resp = c
        .get(format!("{url}/api/v1/activities/{activity_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "get activity");
    let activity: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(activity["status"], "PENDING");
    assert_eq!(activity["name"], "fetch");
    assert_eq!(activity["task_queue"], "default");
    assert_eq!(activity["workflow_id"], "wf-1");
    assert_eq!(activity["seq"], 1);

    // 4. Workflow event log has WorkflowStarted + ActivityScheduled
    let resp = c
        .get(format!("{url}/api/v1/workflows/wf-1/events"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "get events");
    let events: Vec<serde_json::Value> = resp.json().await.unwrap();
    let types: Vec<&str> = events.iter().map(|e| e["event_type"].as_str().unwrap()).collect();
    assert!(
        types.contains(&"WorkflowStarted"),
        "events should include WorkflowStarted, got {types:?}"
    );
    assert!(
        types.contains(&"ActivityScheduled"),
        "events should include ActivityScheduled, got {types:?}"
    );

    // 5. Workflow status is now RUNNING (was PENDING)
    let resp = c
        .get(format!("{url}/api/v1/workflows/wf-1"))
        .send()
        .await
        .unwrap();
    let wf: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(wf["status"], "RUNNING");
}

/// 9.1.6 — Idempotency: scheduling the same (workflow_id, seq) twice returns
/// the same activity id and does NOT create a second row or event.
#[tokio::test]
async fn schedule_activity_is_idempotent_on_seq() {
    let (url, _h) = start_test_server().await;
    let c = client();

    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "TestWorkflow",
            "workflow_id": "wf-idem",
            "task_queue": "default",
        }))
        .send()
        .await
        .unwrap();

    let body = serde_json::json!({
        "name": "fetch",
        "input": {"x": 1},
        "task_queue": "default",
        "seq": 1,
    });

    let r1: serde_json::Value = c
        .post(format!("{url}/api/v1/workflows/wf-idem/activities"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let r2: serde_json::Value = c
        .post(format!("{url}/api/v1/workflows/wf-idem/activities"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(r1["id"], r2["id"], "same seq → same activity id");

    // Only one ActivityScheduled event should have been appended
    let events: Vec<serde_json::Value> = c
        .get(format!("{url}/api/v1/workflows/wf-idem/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let scheduled_count = events
        .iter()
        .filter(|e| e["event_type"].as_str() == Some("ActivityScheduled"))
        .count();
    assert_eq!(scheduled_count, 1, "second schedule must not append a second event");
}
