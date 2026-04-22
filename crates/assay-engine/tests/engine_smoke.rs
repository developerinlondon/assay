//! Phase-3 integration test.
//!
//! Spawns the `assay-engine` binary against a temp SQLite DB on a random
//! free port, polls `/api/v1/health` until ready, then exercises the key
//! endpoints: health, version, namespaces, dashboard index. Confirms
//! shape of the responses.
//!
//! This test proves the whole binary wires together correctly — config
//! parsing → backend connect → migrations → axum compose → router
//! serving → both workflow API and dashboard paths answering.

use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Find a free TCP port by binding to 127.0.0.1:0, reading the assigned
/// port, and dropping the listener. Race-prone in theory; fine for tests
/// that spawn immediately.
fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    l.local_addr().unwrap().port()
}

fn engine_binary() -> PathBuf {
    // cargo test sets CARGO_BIN_EXE_<name> for integration tests.
    env!("CARGO_BIN_EXE_assay-engine").into()
}

struct EngineProcess {
    child: Child,
    port: u16,
    _tmpdir: tempfile::TempDir,
}

impl EngineProcess {
    fn spawn() -> Self {
        let port = free_port();
        let tmp = tempfile::tempdir().expect("tempdir");
        let db_path = tmp.path().join("engine.db");
        let cfg_path = tmp.path().join("engine.toml");

        let cfg = format!(
            r#"
[server]
bind_addr = "127.0.0.1:{port}"

[backend]
type = "sqlite"
path = "{db}"

[logging]
level = "error"
format = "pretty"
"#,
            db = db_path.display()
        );
        std::fs::write(&cfg_path, cfg).expect("write cfg");

        let child = Command::new(engine_binary())
            .arg("serve")
            .arg("--config")
            .arg(&cfg_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn engine");

        Self {
            child,
            port,
            _tmpdir: tmp,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    async fn wait_ready(&self, client: &reqwest::Client) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(r) = client.get(self.url("/api/v1/health")).send().await
                && r.status().is_success()
            {
                return;
            }
            if Instant::now() >= deadline {
                panic!("engine did not become ready on port {}", self.port);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

impl Drop for EngineProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn engine_smoke_sqlite() {
    // Log to stderr so test output shows the port if anything fails.
    let _ = writeln!(std::io::stderr(), "engine_smoke_sqlite starting");

    let engine = EngineProcess::spawn();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    engine.wait_ready(&client).await;

    // ── /api/v1/health ────────────────────────────────────────────────
    let r = client.get(engine.url("/api/v1/health")).send().await.unwrap();
    assert_eq!(r.status(), 200, "health should return 200");
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["service"], "assay-workflow");
    assert_eq!(body["status"], "ok");

    // ── /api/v1/version ───────────────────────────────────────────────
    let r = client.get(engine.url("/api/v1/version")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert!(
        body.get("version").is_some(),
        "version response should have `version` field"
    );

    // ── /api/v1/namespaces ────────────────────────────────────────────
    let r = client
        .get(engine.url("/api/v1/namespaces"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    let arr = body.as_array().expect("namespaces should be a JSON array");
    assert!(
        arr.iter().any(|n| n["name"] == "main"),
        "`main` namespace should be auto-seeded on first connect"
    );

    // ── /workflow/ (dashboard index) ──────────────────────────────────
    let r = client.get(engine.url("/workflow/")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let ct = r
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        ct.starts_with("text/html"),
        "dashboard should return text/html, got {ct}"
    );
    let body = r.text().await.unwrap();
    assert!(
        body.contains("<html") || body.contains("<!DOCTYPE") || body.contains("<!doctype"),
        "dashboard body should contain HTML doctype"
    );

    // ── / (root → redirect to /workflow/) ─────────────────────────────
    let r = client
        .get(engine.url("/"))
        .send()
        .await
        .unwrap();
    assert!(
        r.status().is_success() || r.status().is_redirection(),
        "root should be 2xx or 3xx (redirect), got {}",
        r.status()
    );
}
