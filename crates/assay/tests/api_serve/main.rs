// Drives the real `assay api-serve` process over a socket, so the gate the
// server actually presents to the network is what gets exercised.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const TOKEN: &str = "test-token-a";

fn assay_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_assay"))
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    listener.local_addr().expect("addr").port()
}

struct Server {
    child: Child,
    base: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn start_server(state_dir: &std::path::Path) -> Server {
    let port = free_port();
    let child = Command::new(assay_binary())
        .arg("api-serve")
        .arg("--bind")
        .arg(format!("127.0.0.1:{port}"))
        .env("ASSAY_API_TOKENS", TOKEN)
        .env("ASSAY_STATE_DIR", state_dir)
        .env_remove("ASSAY_POLICY_FILE")
        .env_remove("ASSAY_READONLY")
        .env_remove("ASSAY_APPROVAL")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn api-serve");

    // Wrapped before the readiness loop so the Drop guard reaps the child
    // even when the server never comes up and we panic.
    let server = Server {
        child,
        base: format!("http://127.0.0.1:{port}"),
    };

    let client = reqwest::Client::new();
    for _ in 0..100 {
        if client
            .get(format!("{}/healthz", server.base))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            return server;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("api-serve never became healthy on {}", server.base);
}

async fn post(
    base: &str,
    path: &str,
    token: Option<&str>,
    body: serde_json::Value,
) -> (u16, String) {
    let client = reqwest::Client::new();
    let mut request = client.post(format!("{base}{path}")).json(&body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.expect("request");
    let status = response.status().as_u16();
    (status, response.text().await.unwrap_or_default())
}

/// Start a server and post one run against it, returning (status, body).
async fn run_once(token: Option<&str>, script: &str, mode: &str) -> (u16, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let server = start_server(dir.path()).await;
    post(
        &server.base,
        "/v1/run",
        token,
        serde_json::json!({ "script": script, "mode": mode }),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unauthenticated_run_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let server = start_server(dir.path()).await;

    let (status, _) = post(
        &server.base,
        "/v1/run",
        None,
        serde_json::json!({
            "script": "return 1",
            "mode": "readonly",
        }),
    )
    .await;
    assert_eq!(status, 401);

    let (status, _) = post(
        &server.base,
        "/v1/run",
        Some("wrong"),
        serde_json::json!({
            "script": "return 1",
            "mode": "readonly",
        }),
    )
    .await;
    assert_eq!(status, 401, "a wrong token must not be accepted");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_authenticated_run_returns_the_tool_envelope() {
    let (status, body) = run_once(Some(TOKEN), "return { n = 40 + 2 }", "readonly").await;

    assert_eq!(status, 200, "body: {body}");
    let json: serde_json::Value = serde_json::from_str(&body).expect("envelope json");
    assert_eq!(json["status"], "ok", "body: {body}");
    assert_eq!(json["output"]["n"], 42);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_mode_gate_still_applies_over_http() {
    let (status, body) = run_once(
        Some(TOKEN),
        "http.post(\"http://127.0.0.1:1/x\", \"{}\") return 1",
        "readonly",
    )
    .await;

    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_str(&body).expect("envelope json");
    assert_eq!(json["status"], "error");
    let err = json["error"].as_str().unwrap_or_default();
    assert!(err.contains("readonly: http.post blocked"), "got: {err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unrestricted_is_refused_unless_enabled() {
    let (status, body) = run_once(Some(TOKEN), "return 1", "unrestricted").await;

    assert_eq!(status, 400, "body: {body}");
    assert!(body.contains("not enabled on this server"), "got: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_runs_do_not_interfere() {
    let dir = tempfile::tempdir().expect("tempdir");
    let server = start_server(dir.path()).await;

    let calls = (1..=6).map(|n| {
        let base = server.base.clone();
        async move {
            let (status, body) = post(
                &base,
                "/v1/run",
                Some(TOKEN),
                serde_json::json!({
                    "script": format!("return {{ n = {n} * 2 }}"),
                    "mode": "readonly",
                }),
            )
            .await;
            assert_eq!(status, 200, "body: {body}");
            let json: serde_json::Value = serde_json::from_str(&body).expect("json");
            assert_eq!(json["output"]["n"], n * 2, "body: {body}");
        }
    });
    futures_util::future::join_all(calls).await;
}

#[test]
fn the_server_refuses_to_start_without_tokens() {
    let output = Command::new(assay_binary())
        .arg("api-serve")
        .arg("--bind")
        .arg("127.0.0.1:0")
        .env_remove("ASSAY_API_TOKENS")
        .output()
        .expect("run api-serve");

    assert_eq!(
        output.status.code(),
        Some(2),
        "an ungated runtime must not serve"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to serve an ungated runtime"),
        "got: {stderr}"
    );
}
