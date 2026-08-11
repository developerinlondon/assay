// A grant is only as strong as what it is bound to. These drive the
// spawned binary through suspend → tamper → resume, so the binding is
// exercised across the process boundary the resume flow actually crosses.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

static DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

fn assay_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_assay"))
}

fn unique_dir(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("{prefix}-{nonce}-{seq}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn scrub(cmd: &mut Command) {
    for key in [
        "ASSAY_APPROVAL",
        "ASSAY_APPROVED_INDICES",
        "ASSAY_APPROVED_OPS",
        "ASSAY_DENIED_INDEX",
        "ASSAY_READONLY",
        "ASSAY_MODE",
        "ASSAY_APPROVAL_RESULT",
    ] {
        cmd.env_remove(key);
    }
}

fn tool_cmd(script: &Path, state_dir: &Path) -> Command {
    let mut cmd = Command::new(assay_binary());
    cmd.arg("run")
        .arg("--mode")
        .arg("tool")
        .arg("--approval-mode")
        .arg(script);
    scrub(&mut cmd);
    cmd.env("ASSAY_STATE_DIR", state_dir);
    cmd
}

fn resume_cmd(token: &str, state_dir: &Path) -> Command {
    let mut cmd = Command::new(assay_binary());
    cmd.arg("resume")
        .arg("--token")
        .arg(token)
        .arg("--approve")
        .arg("yes");
    scrub(&mut cmd);
    cmd.env("ASSAY_STATE_DIR", state_dir);
    cmd
}

async fn run_blocking(mut cmd: Command) -> std::process::Output {
    tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap()
}

fn stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout not JSON ({e}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn post_script(url: &str, body: &str) -> String {
    format!("local r = http.post(\"{url}\", {body})\nreturn {{ status = r.status }}")
}

/// Suspend a fresh run at its first mutation and hand back the token.
async fn suspend(dir: &Path, script: &Path) -> String {
    let json = stdout_json(&run_blocking(tool_cmd(script, dir)).await);
    assert_eq!(
        json["status"], "needs_approval",
        "expected a suspend: {json}"
    );
    json["requiresApproval"]["resumeToken"]
        .as_str()
        .expect("resume token")
        .to_string()
}

async fn mock_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/submit"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/elsewhere"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    server
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_approval_payload_carries_a_digest_and_header_names_only() {
    let dir = unique_dir("assay-bind-payload");
    let server = mock_server().await;
    let script = dir.join("s.lua");
    std::fs::write(
        &script,
        format!(
            "local r = http.post(\"{}/submit\", \"{{}}\", {{ headers = {{ Authorization = \"Bearer s3cret-token\" }} }})\nreturn {{ status = r.status }}",
            server.uri()
        ),
    )
    .unwrap();

    let out = run_blocking(tool_cmd(&script, &dir)).await;
    let json = stdout_json(&out);
    let requires = &json["requiresApproval"];

    assert_eq!(json["status"], "needs_approval");
    let digest = requires["digest"].as_str().expect("a digest is present");
    assert_eq!(digest.len(), 64, "expected a sha256 hex digest: {digest}");
    assert_eq!(
        requires["headers"],
        serde_json::json!(["Authorization"]),
        "header names identify the credential in play"
    );

    let rendered = serde_json::to_string(requires).unwrap();
    assert!(
        !rendered.contains("s3cret-token"),
        "a header value must never reach the approval payload: {rendered}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resume_whose_request_changed_is_refused() {
    let dir = unique_dir("assay-bind-tamper");
    let server = mock_server().await;
    let script = dir.join("s.lua");
    std::fs::write(
        &script,
        post_script(&format!("{}/submit", server.uri()), "\"{}\""),
    )
    .unwrap();

    let token = suspend(&dir, &script).await;

    // Same op at the same index, different target: the grant must not
    // stretch to cover it.
    std::fs::write(
        &script,
        post_script(&format!("{}/elsewhere", server.uri()), "\"{}\""),
    )
    .unwrap();

    let resumed = stdout_json(&run_blocking(resume_cmd(&token, &dir)).await);
    assert_eq!(resumed["status"], "error", "tampered resume must not run");
    let err = resumed["error"].as_str().unwrap_or_default();
    assert!(
        err.contains("changed since approval"),
        "expected a binding refusal, got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resume_whose_body_changed_is_refused() {
    let dir = unique_dir("assay-bind-body");
    let server = mock_server().await;
    let script = dir.join("s.lua");
    let url = format!("{}/submit", server.uri());
    std::fs::write(&script, post_script(&url, "{ amount = 1 }")).unwrap();

    let token = suspend(&dir, &script).await;
    std::fs::write(&script, post_script(&url, "{ amount = 1000000 }")).unwrap();

    let resumed = stdout_json(&run_blocking(resume_cmd(&token, &dir)).await);
    assert_eq!(resumed["status"], "error");
    let err = resumed["error"].as_str().unwrap_or_default();
    assert!(
        err.contains("changed since approval"),
        "a changed body must invalidate the grant, got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_untampered_resume_still_runs() {
    let dir = unique_dir("assay-bind-clean");
    let server = mock_server().await;
    let script = dir.join("s.lua");
    std::fs::write(
        &script,
        post_script(&format!("{}/submit", server.uri()), "\"{}\""),
    )
    .unwrap();

    let token = suspend(&dir, &script).await;
    let resumed = stdout_json(&run_blocking(resume_cmd(&token, &dir)).await);
    assert_eq!(
        resumed["status"], "ok",
        "an unchanged replay must still be admitted: {resumed}"
    );
    assert_eq!(resumed["output"]["status"], 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_grant_without_a_digest_is_refused() {
    let dir = unique_dir("assay-bind-legacy");
    let server = mock_server().await;
    let script = dir.join("s.lua");
    std::fs::write(
        &script,
        post_script(&format!("{}/submit", server.uri()), "\"{}\""),
    )
    .unwrap();

    let token = suspend(&dir, &script).await;

    // Simulate resume state written before request binding existed.
    let state_path = dir.join("resume").join(format!("{token}.json"));
    let mut state: Value =
        serde_json::from_slice(&std::fs::read(&state_path).unwrap()).expect("state json");
    state.as_object_mut().unwrap().remove("digest");
    std::fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();

    let resumed = stdout_json(&run_blocking(resume_cmd(&token, &dir)).await);
    assert_eq!(resumed["status"], "error", "must fail closed");
    let err = resumed["error"].as_str().unwrap_or_default();
    assert!(
        err.contains("predates request binding"),
        "expected a fail-closed refusal, got: {err}"
    );
}
