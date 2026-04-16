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

/// Helper used by 9.2 tests: schedule a workflow + activity, claim it as a
/// fake worker, and return the activity id ready to be completed/failed.
async fn schedule_and_claim(c: &reqwest::Client, url: &str, workflow_id: &str) -> i64 {
    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "TestWorkflow",
            "workflow_id": workflow_id,
            "task_queue": "default",
        }))
        .send()
        .await
        .unwrap();

    c.post(format!("{url}/api/v1/workers/register"))
        .json(&serde_json::json!({
            "identity": "test-worker",
            "queue": "default",
            "activities": ["fetch"],
        }))
        .send()
        .await
        .unwrap();

    let scheduled: serde_json::Value = c
        .post(format!("{url}/api/v1/workflows/{workflow_id}/activities"))
        .json(&serde_json::json!({
            "name": "fetch",
            "input": {"x": 1},
            "task_queue": "default",
            "seq": 1,
            "max_attempts": 3,
            "initial_interval_secs": 0.05,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let activity_id = scheduled["id"].as_i64().expect("activity id");

    // Claim via /tasks/poll so worker has the activity in RUNNING state
    let poll_resp: serde_json::Value = c
        .post(format!("{url}/api/v1/tasks/poll"))
        .json(&serde_json::json!({
            "queue": "default",
            "worker_id": "test-worker",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        poll_resp["id"].as_i64(),
        Some(activity_id),
        "expected to claim the just-scheduled activity, got {poll_resp}"
    );

    activity_id
}

/// 9.2 — completing an activity appends ActivityCompleted to the workflow
/// event log with the activity's seq, and the workflow record stays in
/// RUNNING (not COMPLETED — that needs orchestration to know there's no
/// more work).
#[tokio::test]
async fn complete_activity_appends_event() {
    let (url, _h) = start_test_server().await;
    let c = client();
    let activity_id = schedule_and_claim(&c, &url, "wf-complete").await;

    let resp = c
        .post(format!("{url}/api/v1/tasks/{activity_id}/complete"))
        .json(&serde_json::json!({"result": {"bytes": 42}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let events: Vec<serde_json::Value> = c
        .get(format!("{url}/api/v1/workflows/wf-complete/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let completed = events
        .iter()
        .find(|e| e["event_type"].as_str() == Some("ActivityCompleted"))
        .expect("ActivityCompleted event should appear");
    let payload: serde_json::Value =
        serde_json::from_str(completed["payload"].as_str().unwrap()).unwrap();
    assert_eq!(payload["activity_seq"], 1, "event must carry activity seq");
    assert_eq!(payload["activity_id"], activity_id);
    assert!(payload["result"].is_object() || payload["result"].is_string());
}

/// 9.2 — fail_activity with retry policy: first failure re-queues with
/// backoff (status returns to PENDING with attempt+=1); the workflow only
/// gets ActivityFailed once attempts are exhausted.
#[tokio::test]
async fn fail_activity_retries_until_max_attempts() {
    let (url, _h) = start_test_server().await;
    let c = client();
    let activity_id = schedule_and_claim(&c, &url, "wf-retry").await;

    // First failure → should re-queue (attempts left)
    let resp = c
        .post(format!("{url}/api/v1/tasks/{activity_id}/fail"))
        .json(&serde_json::json!({"error": "transient: ConnectionReset"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Activity should be PENDING again with attempt = 2
    let act: serde_json::Value = c
        .get(format!("{url}/api/v1/activities/{activity_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        act["status"], "PENDING",
        "first fail must requeue while attempts remain, got {act}"
    );
    assert_eq!(act["attempt"], 2, "attempt should increment");

    // No ActivityFailed event yet
    let events: Vec<serde_json::Value> = c
        .get(format!("{url}/api/v1/workflows/wf-retry/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let failed_count = events
        .iter()
        .filter(|e| e["event_type"].as_str() == Some("ActivityFailed"))
        .count();
    assert_eq!(failed_count, 0, "should not fire ActivityFailed while retrying");

    // Wait for the backoff to elapse, then claim + fail attempts 2 and 3
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    for expected_attempt in 2..=3 {
        let claimed: serde_json::Value = c
            .post(format!("{url}/api/v1/tasks/poll"))
            .json(&serde_json::json!({
                "queue": "default",
                "worker_id": "test-worker",
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(claimed["id"].as_i64(), Some(activity_id), "should re-claim same activity");
        assert_eq!(claimed["attempt"], expected_attempt);

        c.post(format!("{url}/api/v1/tasks/{activity_id}/fail"))
            .json(&serde_json::json!({"error": "still failing"}))
            .send()
            .await
            .unwrap();

        if expected_attempt < 3 {
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        }
    }

    // Now the activity should be permanently FAILED with one ActivityFailed event
    let act: serde_json::Value = c
        .get(format!("{url}/api/v1/activities/{activity_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(act["status"], "FAILED", "after max attempts the activity is FAILED");
    assert_eq!(act["attempt"], 3);

    let events: Vec<serde_json::Value> = c
        .get(format!("{url}/api/v1/workflows/wf-retry/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let failed_count = events
        .iter()
        .filter(|e| e["event_type"].as_str() == Some("ActivityFailed"))
        .count();
    assert_eq!(failed_count, 1, "exactly one ActivityFailed event after exhausting retries");
}

// ─── 9.3 — Workflow task dispatch loop ────────────────────────────────────
//
// A "workflow task" represents "this workflow has new events that need a
// worker to run the workflow handler against." It's distinct from an
// "activity task" which runs the concrete activity code. Dispatch is the
// loop: start_workflow / activity-complete / timer-fire / signal-arrive
// each set the workflow's needs_dispatch flag, a worker polls
// /workflow-tasks/poll, runs the handler, posts new commands, releases.

/// Helper: poll a workflow task and return the JSON response body, or null
/// when nothing's available.
async fn poll_workflow_task(
    c: &reqwest::Client,
    url: &str,
    queue: &str,
    worker_id: &str,
) -> serde_json::Value {
    c.post(format!("{url}/api/v1/workflow-tasks/poll"))
        .json(&serde_json::json!({"queue": queue, "worker_id": worker_id}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// 9.3 — A freshly-started workflow becomes immediately dispatchable, and
/// the poll response carries the workflow id, type, input, and full event
/// history so a worker can replay deterministically.
#[tokio::test]
async fn start_workflow_makes_it_dispatchable() {
    let (url, _h) = start_test_server().await;
    let c = client();

    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "TestWorkflow",
            "workflow_id": "wf-disp-1",
            "task_queue": "default",
            "input": {"hello": "world"},
        }))
        .send()
        .await
        .unwrap();

    let task = poll_workflow_task(&c, &url, "default", "worker-A").await;
    assert_eq!(task["workflow_id"], "wf-disp-1");
    assert_eq!(task["workflow_type"], "TestWorkflow");
    assert_eq!(task["input"]["hello"], "world");
    let history = task["history"].as_array().expect("history is an array");
    assert!(
        history.iter().any(|e| e["event_type"] == "WorkflowStarted"),
        "history should include WorkflowStarted, got {history:?}"
    );
}

/// 9.3 — A workflow task is claimable only once until the worker
/// releases it (commits commands or its lease ages out). The second
/// poller from the same queue must get null.
#[tokio::test]
async fn workflow_task_claim_is_exclusive() {
    let (url, _h) = start_test_server().await;
    let c = client();

    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "TestWorkflow",
            "workflow_id": "wf-disp-2",
            "task_queue": "default",
        }))
        .send()
        .await
        .unwrap();

    let first = poll_workflow_task(&c, &url, "default", "worker-A").await;
    assert_eq!(first["workflow_id"], "wf-disp-2", "worker-A should claim it");

    let second = poll_workflow_task(&c, &url, "default", "worker-B").await;
    assert!(second.is_null(), "worker-B must get nothing while worker-A holds it");
}

/// 9.3 — Submitting commands releases the claim. The worker submits a
/// `ScheduleActivity` command; the engine schedules the activity and
/// removes the workflow from the dispatchable pool until the activity
/// completes.
#[tokio::test]
async fn submit_commands_schedules_activities_and_releases_claim() {
    let (url, _h) = start_test_server().await;
    let c = client();

    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "TestWorkflow",
            "workflow_id": "wf-disp-3",
            "task_queue": "default",
        }))
        .send()
        .await
        .unwrap();

    let _claim = poll_workflow_task(&c, &url, "default", "worker-A").await;

    // Worker submits a ScheduleActivity command at seq 1
    let resp = c
        .post(format!("{url}/api/v1/workflow-tasks/wf-disp-3/commands"))
        .json(&serde_json::json!({
            "worker_id": "worker-A",
            "commands": [
                {"type": "ScheduleActivity", "seq": 1, "name": "fetch",
                 "task_queue": "default", "input": {"k": "v"}}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Activity should now exist with seq 1
    let events: Vec<serde_json::Value> = c
        .get(format!("{url}/api/v1/workflows/wf-disp-3/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        events.iter().any(|e| e["event_type"] == "ActivityScheduled"),
        "command should have produced ActivityScheduled"
    );

    // Workflow is no longer dispatchable (it's waiting on the activity)
    let next = poll_workflow_task(&c, &url, "default", "worker-A").await;
    assert!(
        next.is_null(),
        "workflow should not be re-dispatchable until something new happens"
    );
}

/// 9.3 — When an activity completes, the workflow becomes dispatchable
/// again so the worker can replay and decide what to do next.
#[tokio::test]
async fn activity_completion_redispatches_workflow() {
    let (url, _h) = start_test_server().await;
    let c = client();

    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "TestWorkflow",
            "workflow_id": "wf-disp-4",
            "task_queue": "default",
        }))
        .send()
        .await
        .unwrap();
    poll_workflow_task(&c, &url, "default", "worker-A").await;

    // Schedule + claim + complete an activity (mirrors a real worker loop)
    let scheduled: serde_json::Value = c
        .post(format!("{url}/api/v1/workflows/wf-disp-4/activities"))
        .json(&serde_json::json!({
            "name": "fetch", "seq": 1, "task_queue": "default", "input": {}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let activity_id = scheduled["id"].as_i64().unwrap();
    c.post(format!("{url}/api/v1/workers/register"))
        .json(&serde_json::json!({
            "identity": "act-worker", "queue": "default", "activities": ["fetch"],
        }))
        .send()
        .await
        .unwrap();
    c.post(format!("{url}/api/v1/tasks/poll"))
        .json(&serde_json::json!({"queue": "default", "worker_id": "act-worker"}))
        .send()
        .await
        .unwrap();
    c.post(format!("{url}/api/v1/tasks/{activity_id}/complete"))
        .json(&serde_json::json!({"result": {"ok": true}}))
        .send()
        .await
        .unwrap();

    // The workflow should now be claimable again — the worker (which had
    // submitted commands and released its claim) needs to replay.
    // First release worker-A's claim by submitting an empty commands batch:
    c.post(format!("{url}/api/v1/workflow-tasks/wf-disp-4/commands"))
        .json(&serde_json::json!({"worker_id": "worker-A", "commands": []}))
        .send()
        .await
        .unwrap();

    let next = poll_workflow_task(&c, &url, "default", "worker-A").await;
    assert_eq!(
        next["workflow_id"], "wf-disp-4",
        "ActivityCompleted should make the workflow dispatchable again, got {next}"
    );
}

/// 9.3 — A CompleteWorkflow command marks the workflow COMPLETED, writes
/// the result, and removes it from the dispatchable pool permanently.
#[tokio::test]
async fn complete_workflow_command_marks_terminal() {
    let (url, _h) = start_test_server().await;
    let c = client();

    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "TestWorkflow",
            "workflow_id": "wf-disp-5",
            "task_queue": "default",
        }))
        .send()
        .await
        .unwrap();
    poll_workflow_task(&c, &url, "default", "worker-A").await;

    let resp = c
        .post(format!("{url}/api/v1/workflow-tasks/wf-disp-5/commands"))
        .json(&serde_json::json!({
            "worker_id": "worker-A",
            "commands": [
                {"type": "CompleteWorkflow", "result": {"steps": 0}}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let wf: serde_json::Value = c
        .get(format!("{url}/api/v1/workflows/wf-disp-5"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(wf["status"], "COMPLETED");
    let result_str = wf["result"].as_str().expect("result string");
    let result: serde_json::Value = serde_json::from_str(result_str).unwrap();
    assert_eq!(result["steps"], 0);

    // No longer dispatchable
    let next = poll_workflow_task(&c, &url, "default", "worker-A").await;
    assert!(next.is_null(), "completed workflow must not poll");
}

// ─── 9.4 — Lua deterministic-replay runtime end-to-end ─────────────────────
//
// These tests boot the engine in-process AND spawn a real assay subprocess
// running the actual stdlib/workflow.lua client. They prove the engine and
// the Lua client interoperate to run a workflow from Pending → Completed
// with real activities that return real values.
//
// The subprocess approach intentionally exercises the same `assay run` path
// users will, so any breakage in the Lua client / API contract is caught.

use std::path::PathBuf;
use std::process::Stdio;

/// Locate the `assay` binary inside the workspace target dir. Tries
/// `debug` then `release`. Returns `None` if neither exists — caller
/// should skip the test with a message in that case (CI builds the
/// binary first; a fresh local checkout might not have it yet).
fn locate_assay_binary() -> Option<PathBuf> {
    let here = std::env::current_dir().ok()?;
    // Walk up until we find a `target` dir (handles running from
    // workspace root or from the crate dir).
    let mut probe = here.clone();
    loop {
        let cand_dbg = probe.join("target/debug/assay");
        let cand_rel = probe.join("target/release/assay");
        if cand_dbg.is_file() {
            return Some(cand_dbg);
        }
        if cand_rel.is_file() {
            return Some(cand_rel);
        }
        if !probe.pop() {
            return None;
        }
    }
}

/// Wait for the workflow to reach a terminal status, polling its REST
/// endpoint. Times out with a useful error containing the last-seen
/// status + recent events for debugging.
async fn wait_for_workflow_status(
    c: &reqwest::Client,
    base_url: &str,
    workflow_id: &str,
    target_status: &str,
    timeout: std::time::Duration,
) -> serde_json::Value {
    let deadline = std::time::Instant::now() + timeout;
    let mut last: serde_json::Value = serde_json::Value::Null;
    while std::time::Instant::now() < deadline {
        let resp = c
            .get(format!("{base_url}/api/v1/workflows/{workflow_id}"))
            .send()
            .await
            .expect("describe workflow");
        if resp.status() == 200 {
            last = resp.json().await.expect("workflow json");
            if last["status"].as_str() == Some(target_status) {
                return last;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let events: serde_json::Value = c
        .get(format!("{base_url}/api/v1/workflows/{workflow_id}/events"))
        .send()
        .await
        .map(|r| r)
        .ok()
        .map(|r| r)
        .unwrap()
        .json()
        .await
        .unwrap_or(serde_json::Value::Null);
    panic!(
        "workflow {workflow_id} did not reach {target_status} within timeout.\n\
         last workflow record: {}\n\
         events: {}",
        serde_json::to_string_pretty(&last).unwrap(),
        serde_json::to_string_pretty(&events).unwrap()
    );
}

/// 9.4 — End-to-end: a real Lua worker subprocess runs a workflow with
/// two sequential activities and the result lands in the workflow record.
#[tokio::test]
async fn lua_workflow_runs_to_completion() {
    let Some(assay_bin) = locate_assay_binary() else {
        eprintln!(
            "SKIP: lua_workflow_runs_to_completion — no assay binary at \
             target/{{debug,release}}/assay. Run `cargo build --bin assay` first."
        );
        return;
    };

    let (url, _h) = start_test_server().await;
    let c = client();

    // Write the worker script to a tempdir
    let tmp = tempfile::tempdir().expect("tempdir");
    let worker_path = tmp.path().join("worker.lua");
    std::fs::write(
        &worker_path,
        r#"
local workflow = require("assay.workflow")
workflow.connect(env.get("ASSAY_ENGINE_URL"))

workflow.define("TwoStep", function(ctx, input)
    local a = ctx:execute_activity("step1", { n = input.n })
    local b = ctx:execute_activity("step2", { prev = a })
    return { first = a, second = b, sum = a.value + b.value }
end)

workflow.activity("step1", function(ctx, input)
    return { value = input.n * 2 }
end)

workflow.activity("step2", function(ctx, input)
    return { value = input.prev.value + 10 }
end)

workflow.listen({ queue = "default" })
"#,
    )
    .expect("write worker.lua");

    // Spawn the assay subprocess
    let mut worker = tokio::process::Command::new(&assay_bin)
        .arg("run")
        .arg(&worker_path)
        .env("ASSAY_ENGINE_URL", &url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn assay worker subprocess");

    // Give the worker a moment to register
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    // Start the workflow
    let resp = c
        .post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "TwoStep",
            "workflow_id": "wf-lua-1",
            "task_queue": "default",
            "input": {"n": 5},
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "workflow start");

    // Wait for completion (generous timeout for slow CI)
    let wf = wait_for_workflow_status(
        &c,
        &url,
        "wf-lua-1",
        "COMPLETED",
        std::time::Duration::from_secs(10),
    )
    .await;

    let result_str = wf["result"].as_str().expect("workflow result");
    let result: serde_json::Value = serde_json::from_str(result_str).expect("result json");

    // step1 doubled n=5 → 10; step2 added 10 → 20; sum = 30
    assert_eq!(result["first"]["value"], 10, "step1 result");
    assert_eq!(result["second"]["value"], 20, "step2 result");
    assert_eq!(result["sum"], 30, "sum");

    // Cleanup
    let _ = worker.kill().await;
}
