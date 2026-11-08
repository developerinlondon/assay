//! The engine runs as PID 1 in its container, where the kernel drops any
//! signal PID 1 has no handler for. An engine that does not handle SIGTERM
//! itself makes every pod stop wait out its whole grace period for the
//! SIGKILL. Outside a container the default action would kill it anyway, so
//! what is pinned is that it exits 0, which only a handled signal does.

mod common;

use common::{EngineProcess, client};
use std::process::Command;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread")]
async fn sigterm_ends_serve_promptly_and_cleanly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = format!(
        "[backend]\ntype = \"sqlite\"\npath = \"{}\"\n",
        dir.path().join("engine.db").display()
    );
    let mut engine = EngineProcess::spawn(dir.path(), "sigterm", &backend);
    engine.wait_ready(&client()).await.expect("engine ready");

    let sent = Command::new("kill")
        .args(["-TERM", &engine.pid().to_string()])
        .status()
        .expect("run kill");
    assert!(sent.success(), "kill -TERM failed");

    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = engine.exited() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "engine still running 10s after SIGTERM: {}",
            engine.log()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(
        status.code(),
        Some(0),
        "SIGTERM should end serve with exit 0, got {status}: {}",
        engine.log()
    );
}
