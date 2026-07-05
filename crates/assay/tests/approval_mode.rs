// Approval execution mode: mutating builtins suspend for per-operation
// approval via the resume flow instead of executing. Activated per-VM via
// `ExecMode::Approval`, process-wide via ASSAY_APPROVAL=1|true, or
// per-invocation via the global `--approval-mode` flag. These tests drive
// the spawned binary so the full suspend → resume → re-run loop is
// exercised end to end.

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

fn write_script(dir: &Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

fn resume_state_path(state_dir: &Path, token: &str) -> PathBuf {
    state_dir.join("resume").join(format!("{token}.json"))
}

fn stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout not JSON ({e}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

// Spawned children must not inherit approval/readonly env from the test
// process; each test adds back exactly what it needs.
fn scrub(cmd: &mut Command) {
    for key in [
        "ASSAY_APPROVAL",
        "ASSAY_APPROVED_INDICES",
        "ASSAY_DENIED_INDEX",
        "ASSAY_READONLY",
        "ASSAY_MODE",
        "ASSAY_APPROVAL_RESULT",
    ] {
        cmd.env_remove(key);
    }
}

fn tool_cmd(script: &Path, flags: &[&str], state_dir: &Path) -> Command {
    let mut cmd = Command::new(assay_binary());
    cmd.arg("run").arg("--mode").arg("tool");
    for flag in flags {
        cmd.arg(flag);
    }
    cmd.arg(script);
    scrub(&mut cmd);
    cmd.env("ASSAY_STATE_DIR", state_dir);
    cmd
}

fn resume_cmd(token: &str, approve: &str, state_dir: &Path) -> Command {
    let mut cmd = Command::new(assay_binary());
    cmd.arg("resume")
        .arg("--token")
        .arg(token)
        .arg("--approve")
        .arg(approve);
    scrub(&mut cmd);
    cmd.env("ASSAY_STATE_DIR", state_dir);
    cmd
}

// Run a blocking spawn off the async runtime so an in-process wiremock
// server stays responsive to the child's requests.
async fn run_blocking(mut cmd: Command) -> std::process::Output {
    tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap()
}

fn post_script(dir: &Path, name: &str, url: &str) -> PathBuf {
    write_script(
        dir,
        name,
        &format!("local r = http.post(\"{url}/submit\", \"{{}}\")\nreturn {{ status = r.status }}"),
    )
}

// ── (a) a mutation halts with a descriptor, without executing ──────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutation_halts_with_descriptor_and_no_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/submit"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;

    let dir = unique_dir("assay-approval-halt");
    let state_dir = dir.join("state");
    let script = post_script(&dir, "post.lua", &server.uri());

    let out = run_blocking(tool_cmd(&script, &["--approval-mode"], &state_dir)).await;
    let json = stdout_json(&out);
    assert_eq!(json["ok"], Value::Bool(true));
    assert_eq!(json["status"], "needs_approval");
    assert_eq!(json["output"], Value::Null);

    let ra = &json["requiresApproval"];
    assert_eq!(ra["op"], "http.post");
    assert_eq!(ra["index"], 0);
    assert!(
        ra["summary"].as_str().unwrap().contains("/submit"),
        "summary: {}",
        ra["summary"]
    );
    let token = ra["resumeToken"].as_str().unwrap();
    assert_eq!(token.len(), 32);
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(resume_state_path(&state_dir, token).exists());

    // The POST never reached the mock.
    assert!(server.received_requests().await.unwrap().is_empty());
}

// ── (b) each resume admits exactly one more operation ──────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_cycle_admits_one_op_per_approval() {
    let server_one = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server_one)
        .await;
    let server_two = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server_two)
        .await;

    let dir = unique_dir("assay-approval-cycle");
    let state_dir = dir.join("state");
    let script = write_script(
        &dir,
        "two_ops.lua",
        &format!(
            "local a = http.post(\"{one}/x\", \"{{}}\")\n\
             local b = http.post(\"{two}/x\", \"{{}}\")\n\
             return {{ one = a.status, two = b.status }}",
            one = server_one.uri(),
            two = server_two.uri(),
        ),
    );

    // Run 1: suspends at op 0.
    let first = run_blocking(tool_cmd(&script, &["--approval-mode"], &state_dir)).await;
    let first_json = stdout_json(&first);
    assert_eq!(first_json["status"], "needs_approval");
    assert_eq!(first_json["requiresApproval"]["index"], 0);
    let token_a = first_json["requiresApproval"]["resumeToken"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(server_one.received_requests().await.unwrap().is_empty());
    assert!(server_two.received_requests().await.unwrap().is_empty());

    // Resume yes: op 0 runs, run re-suspends at op 1.
    let second = run_blocking(resume_cmd(&token_a, "yes", &state_dir)).await;
    let second_json = stdout_json(&second);
    assert_eq!(second_json["status"], "needs_approval");
    assert_eq!(second_json["requiresApproval"]["index"], 1);
    let token_b = second_json["requiresApproval"]["resumeToken"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(!server_one.received_requests().await.unwrap().is_empty());
    assert!(
        server_two.received_requests().await.unwrap().is_empty(),
        "op 1 must not run until approved"
    );

    // Resume yes again: both ops run, script completes.
    let third = run_blocking(resume_cmd(&token_b, "yes", &state_dir)).await;
    let third_json = stdout_json(&third);
    assert_eq!(third_json["status"], "ok");
    assert_eq!(third_json["output"]["one"], 201);
    assert_eq!(third_json["output"]["two"], 202);
    assert!(!server_two.received_requests().await.unwrap().is_empty());
}

// ── (c) deny fails the operation without executing it ──────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deny_fails_operation_and_skips_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/submit"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;

    let dir = unique_dir("assay-approval-deny");
    let state_dir = dir.join("state");
    let script = post_script(&dir, "post.lua", &server.uri());

    let first = run_blocking(tool_cmd(&script, &["--approval-mode"], &state_dir)).await;
    let token = stdout_json(&first)["requiresApproval"]["resumeToken"]
        .as_str()
        .unwrap()
        .to_string();

    let denied = run_blocking(resume_cmd(&token, "no", &state_dir)).await;
    let json = stdout_json(&denied);
    assert_eq!(json["ok"], Value::Bool(false));
    assert_eq!(json["status"], "error");
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("approval: http.post denied"),
        "error: {}",
        json["error"]
    );

    assert!(server.received_requests().await.unwrap().is_empty());
}

// ── (d) read paths run freely without prompting ────────────────────────

#[test]
fn read_only_op_runs_freely_in_approval_mode() {
    let dir = unique_dir("assay-approval-read");
    let state_dir = dir.join("state");
    let data = dir.join("data.txt");
    std::fs::write(&data, "readable payload").unwrap();
    let script = write_script(
        &dir,
        "read.lua",
        &format!("return {{ content = fs.read(\"{}\") }}", data.display()),
    );

    let out = tool_cmd(&script, &["--approval-mode"], &state_dir)
        .output()
        .unwrap();
    let json = stdout_json(&out);
    assert_eq!(json["status"], "ok");
    assert_eq!(json["output"]["content"], "readable payload");
}

// ── (e) flag and env both activate approval mode ───────────────────────

#[test]
fn flag_and_env_activate_approval() {
    let dir = unique_dir("assay-approval-activate");
    let target = dir.join("out.txt");
    let script = write_script(
        &dir,
        "write.lua",
        &format!(
            "fs.write(\"{}\", \"data\")\nreturn {{ done = true }}",
            target.display()
        ),
    );

    // Flag activation.
    let flag_out = tool_cmd(&script, &["--approval-mode"], &dir.join("state-flag"))
        .output()
        .unwrap();
    let flag_json = stdout_json(&flag_out);
    assert_eq!(flag_json["status"], "needs_approval");
    assert_eq!(flag_json["requiresApproval"]["op"], "fs.write");
    assert!(!target.exists(), "fs.write must not run before approval");

    // Env activation.
    let mut env_cmd = tool_cmd(&script, &[], &dir.join("state-env"));
    env_cmd.env("ASSAY_APPROVAL", "1");
    let env_json = stdout_json(&env_cmd.output().unwrap());
    assert_eq!(env_json["status"], "needs_approval");
    assert_eq!(env_json["requiresApproval"]["op"], "fs.write");
}

// ── (f) approval wins over readonly when both are set ──────────────────

#[test]
fn approval_mode_wins_over_readonly() {
    let dir = unique_dir("assay-approval-precedence");
    let state_dir = dir.join("state");
    let target = dir.join("out.txt");
    let script = write_script(
        &dir,
        "write.lua",
        &format!(
            "fs.write(\"{}\", \"data\")\nreturn {{ done = true }}",
            target.display()
        ),
    );

    let out = tool_cmd(&script, &["--readonly", "--approval-mode"], &state_dir)
        .output()
        .unwrap();
    let json = stdout_json(&out);
    assert_eq!(
        json["status"], "needs_approval",
        "approval must win: {json}"
    );
    assert_eq!(json["requiresApproval"]["op"], "fs.write");
    // Not a read-only hard block, and the readonly signal is absent.
    assert!(json.get("error").is_none());
    assert!(json.get("readonly").is_none(), "envelope: {json}");
}

// ── (g) the tool-mode marker does not self-trip the gate ───────────────

#[test]
fn env_marker_does_not_self_trip_in_approval_mode() {
    let dir = unique_dir("assay-approval-marker");
    let state_dir = dir.join("state");
    let script = write_script(
        &dir,
        "marker.lua",
        "return { mode = env.get(\"ASSAY_MODE\") }",
    );

    let out = tool_cmd(&script, &["--approval-mode"], &state_dir)
        .output()
        .unwrap();
    let json = stdout_json(&out);
    assert_eq!(
        json["status"], "ok",
        "marker must not trip the gate: {json}"
    );
    assert_eq!(json["output"]["mode"], "tool");
}

// ── reporting ──────────────────────────────────────────────────────────

#[test]
fn modules_reports_approval_mode() {
    let mut cmd = Command::new(assay_binary());
    cmd.args(["modules", "--approval-mode"]);
    scrub(&mut cmd);
    let out = cmd.output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("approval mode active"), "stdout: {stdout}");
}
