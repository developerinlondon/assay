use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SCRIPT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn assay_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_assay"))
}

fn write_script(name: &str, content: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = SCRIPT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("assay-tool-mode-{nonce}-{seq}"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

fn temp_dir(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = SCRIPT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("{prefix}-{nonce}-{seq}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_tool_mode(script_path: &Path) -> std::process::Output {
    Command::new(assay_binary())
        .args(["run", "--mode", "tool", script_path.to_str().unwrap()])
        .output()
        .unwrap()
}

fn run_tool_mode_with_state_dir(script_path: &Path, state_dir: &Path) -> std::process::Output {
    Command::new(assay_binary())
        .args(["run", "--mode", "tool", script_path.to_str().unwrap()])
        .env("ASSAY_STATE_DIR", state_dir)
        .output()
        .unwrap()
}

fn run_normal_mode(script_path: &Path) -> std::process::Output {
    Command::new(assay_binary())
        .args(["run", script_path.to_str().unwrap()])
        .output()
        .unwrap()
}

fn run_resume(token: &str, approve: &str, state_dir: &Path) -> std::process::Output {
    Command::new(assay_binary())
        .args(["resume", "--token", token, "--approve", approve])
        .env("ASSAY_STATE_DIR", state_dir)
        .output()
        .unwrap()
}

fn approval_script() -> PathBuf {
    write_script(
        "approval.lua",
        r#"
        local oc = require("assay.openclaw")
        local c = oc.client("http://localhost:1", { token = "t" })
        local approved = c:approve("Deploy?", { env = "prod" })
        if approved then
          return { status = "deployed" }
        else
          return { status = "rejected" }
        end
        "#,
    )
}

fn extract_resume_token(json: &Value) -> String {
    json["requiresApproval"]["resumeToken"]
        .as_str()
        .unwrap()
        .to_string()
}

fn resume_state_path(state_dir: &Path, token: &str) -> PathBuf {
    state_dir.join("resume").join(format!("{token}.json"))
}

fn stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn test_tool_mode_success() {
    let script = write_script(
        "success.lua",
        r#"return { message = "ok", values = {1, 2, 3} }"#,
    );

    let output = run_tool_mode(&script);
    assert!(output.status.success());

    let json = stdout_json(&output);
    assert_eq!(json["ok"], Value::Bool(true));
    assert_eq!(json["status"], Value::String("ok".into()));
    assert_eq!(json["requiresApproval"], Value::Null);
    assert_eq!(json["output"]["message"], Value::String("ok".into()));
    assert_eq!(json["output"]["values"], serde_json::json!([1, 2, 3]));
}

#[test]
fn test_tool_mode_error() {
    let script = write_script("error.lua", r#"error("boom")"#);

    let output = run_tool_mode(&script);
    assert!(
        output.status.success(),
        "tool mode should exit zero on envelope"
    );

    let json = stdout_json(&output);
    assert_eq!(json["ok"], Value::Bool(false));
    assert_eq!(json["status"], Value::String("error".into()));
    assert!(json["error"].as_str().unwrap().contains("boom"));
}

#[test]
fn test_tool_mode_nil_return() {
    let script = write_script("nil.lua", "local x = 1\n");

    let output = run_tool_mode(&script);
    assert!(output.status.success());

    let json = stdout_json(&output);
    assert_eq!(json["ok"], Value::Bool(true));
    assert_eq!(json["output"], Value::Null);
}

#[test]
fn test_tool_mode_string_return() {
    let script = write_script("string.lua", r#"return "hello from tool mode""#);

    let output = run_tool_mode(&script);
    assert!(output.status.success());

    let json = stdout_json(&output);
    assert_eq!(json["output"], Value::String("hello from tool mode".into()));
}

#[test]
fn test_tool_mode_env_var() {
    let script = write_script("env.lua", "return { enabled = true }");

    let output = Command::new(assay_binary())
        .args(["run", script.to_str().unwrap()])
        .env("ASSAY_MODE", "tool")
        .output()
        .unwrap();
    assert!(output.status.success());

    let json = stdout_json(&output);
    assert_eq!(json["ok"], Value::Bool(true));
    assert_eq!(json["output"]["enabled"], Value::Bool(true));
}

#[test]
fn test_tool_mode_stderr_separation() {
    let script = write_script(
        "stderr.lua",
        r#"
        log.info("logged to stderr")
        return { ok = true }
        "#,
    );

    let output = run_tool_mode(&script);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("\"output\""),
        "stdout should contain only envelope: {stdout}"
    );
    assert!(
        !stdout.contains("logged to stderr"),
        "stdout should not contain logs: {stdout}"
    );
    assert!(
        stderr.contains("logged to stderr"),
        "stderr should contain logs: {stderr}"
    );
}

#[test]
fn test_tool_mode_timeout() {
    let script = write_script("timeout.lua", "sleep(30)\n");

    let output = Command::new(assay_binary())
        .args([
            "run",
            "--mode",
            "tool",
            "--timeout",
            "2",
            script.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "timeout envelope should still exit zero"
    );

    let json = stdout_json(&output);
    assert_eq!(json["ok"], Value::Bool(false));
    assert_eq!(json["status"], Value::String("timeout".into()));
    assert_eq!(
        json["error"],
        Value::String("execution timed out after 2s".into())
    );
}

#[test]
fn test_tool_mode_truncates_large_output() {
    let payload = "a".repeat(600_000);
    let script = write_script("truncate.lua", &format!(r#"return "{payload}""#));

    let output = run_tool_mode(&script);
    assert!(output.status.success());
    assert!(
        output.stdout.len() <= 524_288,
        "stdout should stay under cap"
    );

    let json = stdout_json(&output);
    assert_eq!(json["ok"], Value::Bool(true));
    assert_eq!(json["truncated"], Value::Bool(true));
    assert!(json["output"].as_str().unwrap().len() < payload.len());
}

#[test]
fn test_tool_mode_approval_halt() {
    let state_dir = temp_dir("assay-resume-state");
    let script = approval_script();

    let output = run_tool_mode_with_state_dir(&script, &state_dir);
    assert!(output.status.success());

    let json = stdout_json(&output);
    assert_eq!(json["ok"], Value::Bool(true));
    assert_eq!(json["status"], Value::String("needs_approval".into()));
    assert_eq!(json["output"], Value::Null);
    assert_eq!(
        json["requiresApproval"]["prompt"],
        Value::String("Deploy?".into())
    );
    assert_eq!(
        json["requiresApproval"]["context"],
        serde_json::json!({ "env": "prod" })
    );

    let token = extract_resume_token(&json);
    assert_eq!(token.len(), 32);
    assert!(token.chars().all(|ch| ch.is_ascii_hexdigit()));
    assert!(resume_state_path(&state_dir, &token).exists());
}

#[test]
fn test_resume_approve_yes() {
    let state_dir = temp_dir("assay-resume-state");
    let script = approval_script();

    let first = run_tool_mode_with_state_dir(&script, &state_dir);
    assert!(first.status.success());
    let first_json = stdout_json(&first);
    let token = extract_resume_token(&first_json);

    let resumed = run_resume(&token, "yes", &state_dir);
    assert!(resumed.status.success());

    let json = stdout_json(&resumed);
    assert_eq!(json["ok"], Value::Bool(true));
    assert_eq!(json["status"], Value::String("ok".into()));
    assert_eq!(json["output"]["status"], Value::String("deployed".into()));
}

#[test]
fn test_resume_approve_no() {
    let state_dir = temp_dir("assay-resume-state");
    let script = approval_script();

    let first = run_tool_mode_with_state_dir(&script, &state_dir);
    assert!(first.status.success());
    let first_json = stdout_json(&first);
    let token = extract_resume_token(&first_json);

    let resumed = run_resume(&token, "no", &state_dir);
    assert!(resumed.status.success());

    let json = stdout_json(&resumed);
    assert_eq!(json["ok"], Value::Bool(true));
    assert_eq!(json["status"], Value::String("ok".into()));
    assert_eq!(json["output"]["status"], Value::String("rejected".into()));
}

#[test]
fn test_resume_expired_token() {
    let state_dir = temp_dir("assay-resume-state");
    let script = approval_script();
    let token = "expiredtokenexpiredtokenexpired12";
    let resume_dir = state_dir.join("resume");
    std::fs::create_dir_all(&resume_dir).unwrap();
    std::fs::write(
        resume_state_path(&state_dir, token),
        serde_json::json!({
            "script_path": script,
            "approval_prompt": "Deploy?",
            "approval_context": { "env": "prod" },
            "created_at": 0,
            "ttl_secs": 1
        })
        .to_string(),
    )
    .unwrap();

    let output = run_resume(token, "yes", &state_dir);
    assert!(output.status.success());

    let json = stdout_json(&output);
    assert_eq!(json["ok"], Value::Bool(false));
    assert_eq!(json["status"], Value::String("error".into()));
    assert!(json["error"].as_str().unwrap().contains("expired"));
}

#[test]
fn test_resume_invalid_token() {
    let state_dir = temp_dir("assay-resume-state");

    let output = run_resume("missing-token", "yes", &state_dir);
    assert!(output.status.success());

    let json = stdout_json(&output);
    assert_eq!(json["ok"], Value::Bool(false));
    assert_eq!(json["status"], Value::String("error".into()));
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("invalid resume token")
    );
}

#[test]
fn test_resume_cleanup() {
    let state_dir = temp_dir("assay-resume-state");
    let script = approval_script();

    let first = run_tool_mode_with_state_dir(&script, &state_dir);
    assert!(first.status.success());
    let first_json = stdout_json(&first);
    let token = extract_resume_token(&first_json);
    let state_path = resume_state_path(&state_dir, &token);
    assert!(state_path.exists());

    let resumed = run_resume(&token, "yes", &state_dir);
    assert!(resumed.status.success());
    assert!(!state_path.exists());
}

#[test]
fn test_tool_mode_normal_mode() {
    let script = write_script(
        "normal.lua",
        r#"
        log.info("plain mode")
        return { ignored = true }
        "#,
    );

    let output = Command::new(assay_binary())
        .args(["run", script.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.trim().is_empty(),
        "normal mode should not emit JSON envelope: {stdout}"
    );
    assert!(
        stderr.contains("plain mode"),
        "normal mode log should remain on stderr: {stderr}"
    );
}

#[test]
fn test_approval_non_tool_mode() {
    let script = approval_script();

    let output = run_normal_mode(&script);
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.trim().is_empty(),
        "normal mode should not emit JSON envelope: {stdout}"
    );
    assert!(stderr.contains("openclaw: approval_required:"));
}
