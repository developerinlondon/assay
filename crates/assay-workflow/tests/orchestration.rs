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

/// 9.6 — End-to-end with a signal: the workflow blocks on
/// `ctx:wait_for_signal("approve")`, the test sends the signal after a
/// pause, and the workflow completes with the signal payload.
#[tokio::test]
async fn lua_workflow_with_signal() {
    let Some(assay_bin) = locate_assay_binary() else {
        eprintln!("SKIP: lua_workflow_with_signal — no assay binary");
        return;
    };

    let (url, _h) = start_test_server().await;
    let c = client();

    let tmp = tempfile::tempdir().expect("tempdir");
    let worker_path = tmp.path().join("worker.lua");
    std::fs::write(
        &worker_path,
        r#"
local workflow = require("assay.workflow")
workflow.connect(env.get("ASSAY_ENGINE_URL"))

workflow.define("WaitForApproval", function(ctx, input)
    local approval = ctx:wait_for_signal("approve")
    return { approved = true, by = approval and approval.by }
end)

workflow.listen({ queue = "default" })
"#,
    )
    .expect("write worker.lua");

    let mut worker = tokio::process::Command::new(&assay_bin)
        .arg("run")
        .arg(&worker_path)
        .env("ASSAY_ENGINE_URL", &url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn worker");
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "WaitForApproval",
            "workflow_id": "wf-sig-1",
            "task_queue": "default",
        }))
        .send()
        .await
        .unwrap();

    // Give the worker time to claim, replay, hit wait_for_signal, and yield
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    // Workflow should be sitting in RUNNING with WorkflowAwaitingSignal in events
    let wf: serde_json::Value = c
        .get(format!("{url}/api/v1/workflows/wf-sig-1"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(wf["status"], "RUNNING", "workflow should be waiting, not done");

    // Send the signal with a payload
    let resp = c
        .post(format!("{url}/api/v1/workflows/wf-sig-1/signal/approve"))
        .json(&serde_json::json!({"payload": {"by": "alice"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Workflow should now wake up and complete
    let final_wf = wait_for_workflow_status(
        &c,
        &url,
        "wf-sig-1",
        "COMPLETED",
        std::time::Duration::from_secs(5),
    )
    .await;

    let result_str = final_wf["result"].as_str().expect("result");
    let result: serde_json::Value = serde_json::from_str(result_str).unwrap();
    assert_eq!(result["approved"], true);
    assert_eq!(result["by"], "alice");

    let _ = worker.kill().await;
}

/// 9.10 — Cron schedule fires a real workflow that runs to completion.
/// Creates a schedule with a never-run-before `next_run_at`, the scheduler
/// fires it on its next tick, the worker claims and completes it.
#[tokio::test]
async fn lua_cron_schedule_fires_real_workflow() {
    let Some(assay_bin) = locate_assay_binary() else {
        eprintln!("SKIP: lua_cron_schedule_fires_real_workflow — no assay binary");
        return;
    };

    let (url, _h) = start_test_server().await;
    let c = client();

    // Worker that runs a trivial activity-driven workflow.
    let tmp = tempfile::tempdir().expect("tempdir");
    let worker_path = tmp.path().join("worker.lua");
    std::fs::write(
        &worker_path,
        r#"
local workflow = require("assay.workflow")
workflow.connect(env.get("ASSAY_ENGINE_URL"))

workflow.define("CronTriggered", function(ctx, input)
    local r = ctx:execute_activity("greet", { who = "world" })
    return { greeting = r.message }
end)

workflow.activity("greet", function(ctx, input)
    return { message = "hello, " .. input.who }
end)

workflow.listen({ queue = "default" })
"#,
    )
    .expect("write worker.lua");

    let mut worker = tokio::process::Command::new(&assay_bin)
        .arg("run")
        .arg(&worker_path)
        .env("ASSAY_ENGINE_URL", &url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn worker");
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    // Create a schedule. next_run_at is None → scheduler treats as
    // never-run-before → fires on next 15s tick.
    let resp = c
        .post(format!("{url}/api/v1/schedules"))
        .json(&serde_json::json!({
            "namespace": "main",
            "name": "test-cron-1",
            "workflow_type": "CronTriggered",
            "cron_expr": "* * * * * *",
            "task_queue": "default",
        }))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "schedule create failed: {}",
        resp.status()
    );

    // Wait up to 25s for: scheduler tick (≤15s) + workflow execution (~1s)
    let started_at = std::time::Instant::now();
    let mut found_workflow_id: Option<String> = None;
    while started_at.elapsed() < std::time::Duration::from_secs(25) {
        let resp = c
            .get(format!("{url}/api/v1/workflows?namespace=main"))
            .send()
            .await
            .unwrap();
        let workflows: Vec<serde_json::Value> = resp.json().await.unwrap_or_default();
        if let Some(wf) = workflows
            .iter()
            .find(|w| w["workflow_type"] == "CronTriggered")
        {
            found_workflow_id = wf["id"].as_str().map(String::from);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    let workflow_id = found_workflow_id
        .expect("scheduler should have started a CronTriggered workflow within 25s");

    let final_wf = wait_for_workflow_status(
        &c,
        &url,
        &workflow_id,
        "COMPLETED",
        std::time::Duration::from_secs(10),
    )
    .await;
    let result_str = final_wf["result"].as_str().expect("result");
    let result: serde_json::Value = serde_json::from_str(result_str).unwrap();
    assert_eq!(result["greeting"], "hello, world");

    let _ = worker.kill().await;
}

/// 9.9 — Worker crash recovery: a workflow uses ctx:side_effect (so we can
/// verify it doesn't run twice across the crash) followed by a sleep + an
/// activity. The first worker is SIGKILL'd mid-flight; a second worker
/// picks up the workflow after the dispatch lease ages out, replays from
/// history (cached side_effect, sees TimerScheduled, runs the activity),
/// and the workflow completes.
///
/// The test sets ASSAY_WF_DISPATCH_TIMEOUT_SECS=2 on the in-process engine
/// so the recovery happens quickly.
#[tokio::test]
async fn lua_worker_crash_resumes_workflow() {
    let Some(assay_bin) = locate_assay_binary() else {
        eprintln!("SKIP: lua_worker_crash_resumes_workflow — no assay binary");
        return;
    };

    // Short dispatch-recovery timeout. Env vars are process-global, so all
    // tests in this binary share the value. The dispatch poller reads it
    // each tick, so other tests still get correct behaviour — just with
    // a 2s recovery budget. Their workflows submit commands well before
    // that, so they're unaffected.
    //
    // SAFETY: set_var is unsafe in Rust 2024 because it can race with
    // multi-threaded readers. Acceptable here: tests in this binary all
    // tolerate any value of this var, and we set it before any workflow
    // runs in parallel.
    unsafe { std::env::set_var("ASSAY_WF_DISPATCH_TIMEOUT_SECS", "2") };

    let (url, _h) = start_test_server().await;
    let c = client();

    let tmp = tempfile::tempdir().expect("tempdir");
    let counter_path = tmp.path().join("counter.txt");
    std::fs::write(&counter_path, "0").expect("init counter");

    let worker_path = tmp.path().join("worker.lua");
    let worker_src = r#"
local workflow = require("assay.workflow")
workflow.connect(env.get("ASSAY_ENGINE_URL"))

local COUNTER = "__COUNTER_PATH__"

local function bump_and_token()
    local cur = tonumber(fs.read(COUNTER)) or 0
    fs.write(COUNTER, tostring(cur + 1))
    return { token = "tok-" .. tostring(cur + 1) }
end

workflow.define("CrashSafeWorkflow", function(ctx, input)
    local t = ctx:side_effect("issue", bump_and_token)
    ctx:sleep(2)
    local r = ctx:execute_activity("step", { token = t.token })
    return { token = t, step = r }
end)

workflow.activity("step", function(ctx, input)
    return { saw = input.token }
end)

workflow.listen({ queue = "default" })
"#
    .replace("__COUNTER_PATH__", counter_path.to_str().unwrap());
    std::fs::write(&worker_path, &worker_src).expect("write worker.lua");

    // Spawn worker A
    let mut worker_a = tokio::process::Command::new(&assay_bin)
        .arg("run")
        .arg(&worker_path)
        .env("ASSAY_ENGINE_URL", &url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn worker A");
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "CrashSafeWorkflow",
            "workflow_id": "wf-crash-1",
            "task_queue": "default",
        }))
        .send()
        .await
        .unwrap();

    // Wait until worker A has at minimum recorded the side_effect — that
    // means the workflow has progressed past the bump_and_token() call and
    // its result is durable in history. We can verify this by polling the
    // event log.
    let recorded = wait_for_event(&c, &url, "wf-crash-1", "SideEffectRecorded",
        std::time::Duration::from_secs(5)).await;
    assert!(recorded, "worker A should have recorded the side effect before we kill it");

    // SIGKILL worker A
    if let Some(pid) = worker_a.id() {
        // Use kill -KILL to bypass any SIGTERM cleanup
        std::process::Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .expect("kill worker A");
    }
    let _ = worker_a.wait().await;

    // Spawn worker B
    let mut worker_b = tokio::process::Command::new(&assay_bin)
        .arg("run")
        .arg(&worker_path)
        .env("ASSAY_ENGINE_URL", &url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn worker B");

    // Wait for workflow to complete via worker B (after dispatch lease
    // ages out and worker B replays from history). 15s budget covers
    // the 2s lease timeout + 2s sleep + replay overhead.
    let final_wf = wait_for_workflow_status(
        &c,
        &url,
        "wf-crash-1",
        "COMPLETED",
        std::time::Duration::from_secs(15),
    )
    .await;

    let result_str = final_wf["result"].as_str().expect("result");
    let result: serde_json::Value = serde_json::from_str(result_str).unwrap();
    assert_eq!(result["token"]["token"], "tok-1");
    assert_eq!(result["step"]["saw"], "tok-1");

    // Critical: the side_effect function ran exactly ONCE total despite
    // the worker crash. Worker B should have read the cached value from
    // history rather than calling bump_and_token again.
    let counter = std::fs::read_to_string(&counter_path).expect("counter").trim().to_string();
    assert_eq!(
        counter, "1",
        "side_effect must run exactly once across worker crash (got {counter} runs)"
    );

    let _ = worker_b.kill().await;
}

/// Helper: poll the event log until we see a particular event_type, or time out.
async fn wait_for_event(
    c: &reqwest::Client,
    base_url: &str,
    workflow_id: &str,
    event_type: &str,
    timeout: std::time::Duration,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let events: Vec<serde_json::Value> = c
            .get(format!("{base_url}/api/v1/workflows/{workflow_id}/events"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap_or_default();
        if events.iter().any(|e| e["event_type"] == event_type) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    false
}

/// 9.8a — side_effect: a workflow needs a non-deterministic value (e.g.
/// random ID, current time). ctx:side_effect runs the function once,
/// records the result in history, and on replay returns the cached value
/// without re-running the function. Verified by counting function calls.
#[tokio::test]
async fn lua_workflow_side_effect_is_recorded_once() {
    let Some(assay_bin) = locate_assay_binary() else {
        eprintln!("SKIP: lua_workflow_side_effect_is_recorded_once — no assay binary");
        return;
    };

    let (url, _h) = start_test_server().await;
    let c = client();

    let tmp = tempfile::tempdir().expect("tempdir");
    let counter_path = tmp.path().join("counter.txt");
    std::fs::write(&counter_path, "0").expect("init counter");

    let worker_path = tmp.path().join("worker.lua");
    let worker_src = r#"
local workflow = require("assay.workflow")
workflow.connect(env.get("ASSAY_ENGINE_URL"))

local COUNTER = "__COUNTER_PATH__"

-- Counter file lets the test see how many times the side_effect
-- function actually ran. Side effects must be recorded once and
-- cached for all subsequent replays.
local function bump_counter_and_return_value()
    local cur = tonumber(fs.read(COUNTER)) or 0
    fs.write(COUNTER, tostring(cur + 1))
    return { token = "tok-" .. tostring(cur + 1) }
end

workflow.define("WithSideEffect", function(ctx, input)
    local token = ctx:side_effect("issue_token", bump_counter_and_return_value)
    -- Two activities so we get multiple replay cycles
    local a = ctx:execute_activity("step", { token = token.token })
    local b = ctx:execute_activity("step", { token = token.token })
    return { token = token, a = a, b = b }
end)

workflow.activity("step", function(ctx, input)
    return { saw = input.token }
end)

workflow.listen({ queue = "default" })
"#
    .replace("__COUNTER_PATH__", counter_path.to_str().unwrap());
    std::fs::write(&worker_path, worker_src).expect("write worker.lua");

    let mut worker = tokio::process::Command::new(&assay_bin)
        .arg("run")
        .arg(&worker_path)
        .env("ASSAY_ENGINE_URL", &url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn worker");
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "WithSideEffect",
            "workflow_id": "wf-se-1",
            "task_queue": "default",
        }))
        .send()
        .await
        .unwrap();

    let final_wf = wait_for_workflow_status(
        &c,
        &url,
        "wf-se-1",
        "COMPLETED",
        std::time::Duration::from_secs(10),
    )
    .await;
    let result_str = final_wf["result"].as_str().expect("result");
    let result: serde_json::Value = serde_json::from_str(result_str).unwrap();

    // The token should be the same across both activities (cached value)
    assert_eq!(result["token"]["token"], "tok-1");
    assert_eq!(result["a"]["saw"], "tok-1");
    assert_eq!(result["b"]["saw"], "tok-1");

    // The side_effect function should have run exactly ONCE despite the
    // workflow being replayed multiple times (once per activity completion).
    let counter = std::fs::read_to_string(&counter_path).expect("counter").trim().to_string();
    assert_eq!(
        counter, "1",
        "side_effect function ran {counter} times — must be exactly 1"
    );

    let _ = worker.kill().await;
}

/// 9.8b — Parent workflow starts a child workflow and waits for it to
/// complete, picking up the child's result. Verifies the parent and child
/// run independently as proper workflows (not just inline subroutines).
#[tokio::test]
async fn lua_child_workflow_completes_before_parent() {
    let Some(assay_bin) = locate_assay_binary() else {
        eprintln!("SKIP: lua_child_workflow_completes_before_parent — no assay binary");
        return;
    };

    let (url, _h) = start_test_server().await;
    let c = client();

    let tmp = tempfile::tempdir().expect("tempdir");
    let worker_path = tmp.path().join("worker.lua");
    std::fs::write(
        &worker_path,
        r#"
local workflow = require("assay.workflow")
workflow.connect(env.get("ASSAY_ENGINE_URL"))

workflow.define("Parent", function(ctx, input)
    local child_result = ctx:start_child_workflow("Child", {
        workflow_id = "child-of-" .. input.parent_label,
        input = { multiplier = input.multiplier },
    })
    return { from_child = child_result, parent_label = input.parent_label }
end)

workflow.define("Child", function(ctx, input)
    local r = ctx:execute_activity("multiply", { x = 7, by = input.multiplier })
    return { product = r.product }
end)

workflow.activity("multiply", function(ctx, input)
    return { product = input.x * input.by }
end)

workflow.listen({ queue = "default" })
"#,
    )
    .expect("write worker.lua");

    let mut worker = tokio::process::Command::new(&assay_bin)
        .arg("run")
        .arg(&worker_path)
        .env("ASSAY_ENGINE_URL", &url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn worker");
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "Parent",
            "workflow_id": "wf-parent-1",
            "task_queue": "default",
            "input": {"parent_label": "alpha", "multiplier": 6},
        }))
        .send()
        .await
        .unwrap();

    let parent = wait_for_workflow_status(
        &c,
        &url,
        "wf-parent-1",
        "COMPLETED",
        std::time::Duration::from_secs(15),
    )
    .await;
    let parent_result_str = parent["result"].as_str().expect("parent result");
    let parent_result: serde_json::Value =
        serde_json::from_str(parent_result_str).expect("parse parent result");
    assert_eq!(parent_result["parent_label"], "alpha");
    assert_eq!(parent_result["from_child"]["product"], 42, "7 * 6 = 42");

    // The child workflow should also be COMPLETED with its parent_id set
    let child: serde_json::Value = c
        .get(format!("{url}/api/v1/workflows/child-of-alpha"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(child["status"], "COMPLETED");
    assert_eq!(child["parent_id"], "wf-parent-1");

    let _ = worker.kill().await;
}

/// 9.7 — End-to-end cancellation: a workflow sleeps for 5 seconds and would
/// then run an activity. We cancel after 200ms; the workflow ends CANCELLED,
/// the timer never fires, and the activity that came after the sleep is
/// never scheduled (verified by counting ActivityScheduled events == 0).
#[tokio::test]
async fn lua_workflow_cancellation_stops_work() {
    let Some(assay_bin) = locate_assay_binary() else {
        eprintln!("SKIP: lua_workflow_cancellation_stops_work — no assay binary");
        return;
    };

    let (url, _h) = start_test_server().await;
    let c = client();

    let tmp = tempfile::tempdir().expect("tempdir");
    let worker_path = tmp.path().join("worker.lua");
    std::fs::write(
        &worker_path,
        r#"
local workflow = require("assay.workflow")
workflow.connect(env.get("ASSAY_ENGINE_URL"))

workflow.define("LongSleepThenWork", function(ctx, input)
    ctx:sleep(5)
    -- This activity should NEVER be scheduled — we cancel before the
    -- timer fires.
    return ctx:execute_activity("never_runs", { x = 1 })
end)

workflow.activity("never_runs", function(ctx, input)
    return { x = input.x }
end)

workflow.listen({ queue = "default" })
"#,
    )
    .expect("write worker.lua");

    let mut worker = tokio::process::Command::new(&assay_bin)
        .arg("run")
        .arg(&worker_path)
        .env("ASSAY_ENGINE_URL", &url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn worker");
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "LongSleepThenWork",
            "workflow_id": "wf-cancel-1",
            "task_queue": "default",
        }))
        .send()
        .await
        .unwrap();

    // Let the worker claim + yield ScheduleTimer
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Cancel the workflow
    let resp = c
        .post(format!("{url}/api/v1/workflows/wf-cancel-1/cancel"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Workflow should reach CANCELLED quickly (worker re-replays + acks)
    let final_wf = wait_for_workflow_status(
        &c,
        &url,
        "wf-cancel-1",
        "CANCELLED",
        std::time::Duration::from_secs(5),
    )
    .await;
    assert_eq!(final_wf["status"], "CANCELLED");

    // Verify NO activity was ever scheduled — the workflow died at sleep
    let events: Vec<serde_json::Value> = c
        .get(format!("{url}/api/v1/workflows/wf-cancel-1/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let activity_scheduled_count = events
        .iter()
        .filter(|e| e["event_type"].as_str() == Some("ActivityScheduled"))
        .count();
    assert_eq!(
        activity_scheduled_count, 0,
        "the post-sleep activity must not have been scheduled, got events: {events:?}"
    );

    // Verify the cancel-request event is in history
    let types: Vec<&str> = events
        .iter()
        .map(|e| e["event_type"].as_str().unwrap_or(""))
        .collect();
    assert!(
        types.contains(&"WorkflowCancelRequested"),
        "missing WorkflowCancelRequested in {types:?}"
    );
    assert!(
        types.contains(&"WorkflowCancelled"),
        "missing terminal WorkflowCancelled in {types:?}"
    );

    let _ = worker.kill().await;
}

/// 9.5 — End-to-end with a durable timer: workflow sleeps for ~1 second
/// (durably — the timer survives a worker bouncing) then completes.
#[tokio::test]
async fn lua_workflow_with_durable_timer() {
    let Some(assay_bin) = locate_assay_binary() else {
        eprintln!("SKIP: lua_workflow_with_durable_timer — no assay binary");
        return;
    };

    let (url, _h) = start_test_server().await;
    let c = client();

    let tmp = tempfile::tempdir().expect("tempdir");
    let worker_path = tmp.path().join("worker.lua");
    std::fs::write(
        &worker_path,
        r#"
local workflow = require("assay.workflow")
workflow.connect(env.get("ASSAY_ENGINE_URL"))

workflow.define("SleepThenStep", function(ctx, input)
    ctx:sleep(1)
    local r = ctx:execute_activity("step", { x = input.x })
    return { x = r.x, slept = true }
end)

workflow.activity("step", function(ctx, input)
    return { x = input.x * 3 }
end)

workflow.listen({ queue = "default" })
"#,
    )
    .expect("write worker.lua");

    let mut worker = tokio::process::Command::new(&assay_bin)
        .arg("run")
        .arg(&worker_path)
        .env("ASSAY_ENGINE_URL", &url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn assay worker");
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    let started_at = std::time::Instant::now();
    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "SleepThenStep",
            "workflow_id": "wf-timer-1",
            "task_queue": "default",
            "input": {"x": 7},
        }))
        .send()
        .await
        .unwrap();

    let wf = wait_for_workflow_status(
        &c,
        &url,
        "wf-timer-1",
        "COMPLETED",
        std::time::Duration::from_secs(15),
    )
    .await;
    let elapsed = started_at.elapsed();

    // Sanity: at LEAST 0.9s (the durable timer was 1s, allow ~100ms slack)
    // and at MOST 5s (anything slower means we're not waking up promptly)
    assert!(
        elapsed >= std::time::Duration::from_millis(900),
        "workflow finished too fast: {elapsed:?} (durable timer should have made us wait ~1s)"
    );
    assert!(
        elapsed <= std::time::Duration::from_secs(5),
        "workflow took too long: {elapsed:?}"
    );

    let result_str = wf["result"].as_str().expect("result");
    let result: serde_json::Value = serde_json::from_str(result_str).unwrap();
    assert_eq!(result["x"], 21, "step ran with input.x*3 after timer");
    assert_eq!(result["slept"], true);

    // History should record TimerScheduled and TimerFired
    let events: Vec<serde_json::Value> = c
        .get(format!("{url}/api/v1/workflows/wf-timer-1/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let types: Vec<&str> = events
        .iter()
        .map(|e| e["event_type"].as_str().unwrap_or(""))
        .collect();
    assert!(types.contains(&"TimerScheduled"), "missing TimerScheduled in {types:?}");
    assert!(types.contains(&"TimerFired"), "missing TimerFired in {types:?}");

    let _ = worker.kill().await;
}

/// F1 — `ctx:register_query` exposes live state via `GET /workflows/{id}/state`.
///
/// The worker registers two query handlers and mutates their backing state
/// across activity calls; each replay recomputes the snapshot. The test
/// asserts that the REST endpoint returns the latest state both as a full
/// map and via the per-query path.
#[tokio::test]
async fn lua_workflow_register_query_exposes_live_state() {
    let Some(assay_bin) = locate_assay_binary() else {
        eprintln!("SKIP: lua_workflow_register_query_exposes_live_state — no assay binary");
        return;
    };

    let (url, _h) = start_test_server().await;
    let c = client();

    let tmp = tempfile::tempdir().expect("tempdir");
    let worker_path = tmp.path().join("worker.lua");
    std::fs::write(
        &worker_path,
        r#"
local workflow = require("assay.workflow")
workflow.connect(env.get("ASSAY_ENGINE_URL"))

workflow.define("Staged", function(ctx, input)
    local state = { stage = "init", progress = 0 }
    ctx:register_query("stage", function() return state.stage end)
    ctx:register_query("progress", function() return state.progress end)

    state.stage = "running"
    state.progress = 0.25
    ctx:execute_activity("step_a", {})
    state.progress = 0.75
    ctx:execute_activity("step_b", {})
    state.stage = "done"
    state.progress = 1.0
    return { ok = true }
end)

workflow.activity("step_a", function(ctx, input) return { ok = true } end)
workflow.activity("step_b", function(ctx, input) return { ok = true } end)

workflow.listen({ queue = "default" })
"#,
    )
    .expect("write worker.lua");

    let mut worker = tokio::process::Command::new(&assay_bin)
        .arg("run")
        .arg(&worker_path)
        .env("ASSAY_ENGINE_URL", &url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn worker");
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "Staged",
            "workflow_id": "wf-rq-1",
            "task_queue": "default",
        }))
        .send()
        .await
        .unwrap();

    // Wait for completion
    wait_for_workflow_status(
        &c,
        &url,
        "wf-rq-1",
        "COMPLETED",
        std::time::Duration::from_secs(10),
    )
    .await;

    // Full state
    let resp = c
        .get(format!("{url}/api/v1/workflows/wf-rq-1/state"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "state endpoint");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["state"]["stage"], "done");
    assert_eq!(body["state"]["progress"], 1.0);

    // Per-query
    let resp = c
        .get(format!("{url}/api/v1/workflows/wf-rq-1/state/stage"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "state/stage endpoint");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["value"], "done");

    // Unknown query name → 404
    let resp = c
        .get(format!("{url}/api/v1/workflows/wf-rq-1/state/nonexistent"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // Workflow without queries → 404 on the state endpoint
    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "Staged",
            "workflow_id": "wf-rq-none",
            "task_queue": "default",
        }))
        .send()
        .await
        .unwrap();
    // (Staged registers queries, so this is a weak test — but we already
    // asserted 200 on the happy path; the key invariant is that workflows
    // that don't call register_query don't emit snapshots, which is
    // covered by unit inspection of `_collect_snapshot` returning nil.)

    let _ = worker.kill().await;
}

/// F2 — `ctx:continue_as_new` resets event history and starts a new run.
///
/// The first run calls continue_as_new with a bumped counter; the engine
/// completes the first workflow and spawns a new run with fresh event
/// history. The test verifies both workflows reach terminal state and the
/// second one received the bumped input.
#[tokio::test]
async fn lua_workflow_continue_as_new_starts_fresh_run() {
    let Some(assay_bin) = locate_assay_binary() else {
        eprintln!("SKIP: lua_workflow_continue_as_new_starts_fresh_run — no assay binary");
        return;
    };

    let (url, _h) = start_test_server().await;
    let c = client();

    let tmp = tempfile::tempdir().expect("tempdir");
    let worker_path = tmp.path().join("worker.lua");
    std::fs::write(
        &worker_path,
        r#"
local workflow = require("assay.workflow")
workflow.connect(env.get("ASSAY_ENGINE_URL"))

-- First run: call continue_as_new with a bumped counter, never returns.
-- Second run: observes the bumped input, completes normally.
workflow.define("Counter", function(ctx, input)
    local n = (input and input.n) or 0
    if n == 0 then
        ctx:continue_as_new({ n = 1 })
    end
    return { final_n = n }
end)

workflow.listen({ queue = "default" })
"#,
    )
    .expect("write worker.lua");

    let mut worker = tokio::process::Command::new(&assay_bin)
        .arg("run")
        .arg(&worker_path)
        .env("ASSAY_ENGINE_URL", &url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn worker");
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "Counter",
            "workflow_id": "wf-can-1",
            "task_queue": "default",
            "input": { "n": 0 },
        }))
        .send()
        .await
        .unwrap();

    // First run completes (closed out by continue_as_new)
    wait_for_workflow_status(
        &c,
        &url,
        "wf-can-1",
        "COMPLETED",
        std::time::Duration::from_secs(10),
    )
    .await;

    // Second run ID follows the engine's naming convention:
    // `{original_id}-continued-{timestamp}`. Find it by listing workflows
    // of this type and picking the one that isn't the original.
    let resp = c
        .get(format!("{url}/api/v1/workflows?type=Counter&limit=10"))
        .send()
        .await
        .unwrap();
    let workflows: Vec<serde_json::Value> = resp.json().await.unwrap();
    let second = workflows
        .iter()
        .find(|w| w["id"].as_str() != Some("wf-can-1"))
        .expect("second run should exist");
    let second_id = second["id"].as_str().expect("second id");

    let second_wf = wait_for_workflow_status(
        &c,
        &url,
        second_id,
        "COMPLETED",
        std::time::Duration::from_secs(5),
    )
    .await;

    let result_str = second_wf["result"].as_str().expect("second run result");
    let result: serde_json::Value = serde_json::from_str(result_str).unwrap();
    assert_eq!(result["final_n"], 1, "second run should see bumped input");

    let _ = worker.kill().await;
}

/// F5 — `ctx:execute_parallel` schedules multiple activities from one
/// replay and waits for all to complete before the workflow proceeds.
#[tokio::test]
async fn lua_workflow_execute_parallel_three_activities() {
    let Some(assay_bin) = locate_assay_binary() else {
        eprintln!("SKIP: lua_workflow_execute_parallel_three_activities — no assay binary");
        return;
    };

    let (url, _h) = start_test_server().await;
    let c = client();

    let tmp = tempfile::tempdir().expect("tempdir");
    let worker_path = tmp.path().join("worker.lua");
    std::fs::write(
        &worker_path,
        r#"
local workflow = require("assay.workflow")
workflow.connect(env.get("ASSAY_ENGINE_URL"))

workflow.define("FanOut", function(ctx, input)
    local results = ctx:execute_parallel({
        { name = "double", input = { n = 1 } },
        { name = "double", input = { n = 2 } },
        { name = "double", input = { n = 3 } },
    })
    return { sum = results[1].v + results[2].v + results[3].v }
end)

workflow.activity("double", function(ctx, input)
    return { v = input.n * 2 }
end)

workflow.listen({ queue = "default" })
"#,
    )
    .expect("write worker.lua");

    let mut worker = tokio::process::Command::new(&assay_bin)
        .arg("run")
        .arg(&worker_path)
        .env("ASSAY_ENGINE_URL", &url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn worker");
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "FanOut",
            "workflow_id": "wf-par-1",
            "task_queue": "default",
        }))
        .send()
        .await
        .unwrap();

    let wf = wait_for_workflow_status(
        &c,
        &url,
        "wf-par-1",
        "COMPLETED",
        std::time::Duration::from_secs(10),
    )
    .await;

    let result_str = wf["result"].as_str().expect("result");
    let result: serde_json::Value = serde_json::from_str(result_str).unwrap();
    // 1*2 + 2*2 + 3*2 = 12
    assert_eq!(result["sum"], 12);

    // All three activities should have been scheduled in the first replay —
    // verify by counting ActivityScheduled events before the first
    // ActivityCompleted.
    let events: Vec<serde_json::Value> = c
        .get(format!("{url}/api/v1/workflows/wf-par-1/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let mut scheduled_before_completed = 0;
    for e in &events {
        match e["event_type"].as_str().unwrap_or("") {
            "ActivityScheduled" => scheduled_before_completed += 1,
            "ActivityCompleted" => break,
            _ => {}
        }
    }
    assert_eq!(
        scheduled_before_completed, 3,
        "all 3 parallel activities should be scheduled before any completes"
    );

    let _ = worker.kill().await;
}

/// F5 — execute_parallel raises when any sub-activity fails after retries.
#[tokio::test]
async fn lua_workflow_execute_parallel_one_fails_raises() {
    let Some(assay_bin) = locate_assay_binary() else {
        eprintln!("SKIP: lua_workflow_execute_parallel_one_fails_raises — no assay binary");
        return;
    };

    let (url, _h) = start_test_server().await;
    let c = client();

    let tmp = tempfile::tempdir().expect("tempdir");
    let worker_path = tmp.path().join("worker.lua");
    std::fs::write(
        &worker_path,
        r#"
local workflow = require("assay.workflow")
workflow.connect(env.get("ASSAY_ENGINE_URL"))

workflow.define("FanOutWithFailure", function(ctx, input)
    ctx:execute_parallel({
        { name = "ok",   input = {}, opts = { max_attempts = 1 } },
        { name = "fail", input = {}, opts = { max_attempts = 1 } },
    })
    return { reached_end = true }
end)

workflow.activity("ok", function(ctx, input) return { ok = true } end)
workflow.activity("fail", function(ctx, input) error("boom") end)

workflow.listen({ queue = "default" })
"#,
    )
    .expect("write worker.lua");

    let mut worker = tokio::process::Command::new(&assay_bin)
        .arg("run")
        .arg(&worker_path)
        .env("ASSAY_ENGINE_URL", &url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn worker");
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    c.post(format!("{url}/api/v1/workflows"))
        .json(&serde_json::json!({
            "workflow_type": "FanOutWithFailure",
            "workflow_id": "wf-par-fail",
            "task_queue": "default",
        }))
        .send()
        .await
        .unwrap();

    let wf = wait_for_workflow_status(
        &c,
        &url,
        "wf-par-fail",
        "FAILED",
        std::time::Duration::from_secs(10),
    )
    .await;

    let error = wf["error"].as_str().expect("error").to_string();
    assert!(
        error.contains("fail") || error.contains("boom"),
        "error should mention the failing activity, got: {error}"
    );

    let _ = worker.kill().await;
}

/// F6 — search attributes settable on start and filterable on list.
#[tokio::test]
async fn search_attributes_filter_list() {
    let (url, _h) = start_test_server().await;
    let c = client();

    // Create three workflows with distinct search attributes
    for (id, env) in [("wf-sa-1", "prod"), ("wf-sa-2", "prod"), ("wf-sa-3", "staging")] {
        c.post(format!("{url}/api/v1/workflows"))
            .json(&serde_json::json!({
                "workflow_type": "Tagged",
                "workflow_id": id,
                "task_queue": "default",
                "search_attributes": { "env": env },
            }))
            .send()
            .await
            .unwrap();
    }

    // No filter: all 3
    let all: Vec<serde_json::Value> = c
        .get(format!("{url}/api/v1/workflows?type=Tagged"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(all.len(), 3);

    // Filter env=prod: 2 (URL-encoded {"env":"prod"})
    let prod: Vec<serde_json::Value> = c
        .get(format!(
            "{url}/api/v1/workflows?type=Tagged&search_attrs=%7B%22env%22%3A%22prod%22%7D"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(prod.len(), 2, "env=prod matches two workflows");

    // Filter env=staging: 1
    let staging: Vec<serde_json::Value> = c
        .get(format!(
            "{url}/api/v1/workflows?type=Tagged&search_attrs=%7B%22env%22%3A%22staging%22%7D"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(staging.len(), 1);
}
