mod common;

use common::create_vm;

#[tokio::test]
async fn test_machine_exec_function_registered() {
    // Smoke test: the function exists. Calling it requires a real nspawn
    // machine, so we only verify it's callable and errors cleanly when
    // the named machine doesn't exist.
    let vm = create_vm();
    let exists: bool = vm
        .load(r#"return type(systemd.machine_exec) == "function""#)
        .eval_async()
        .await
        .unwrap();
    assert!(exists);
}

#[tokio::test]
async fn test_machine_exec_nonexistent_machine_errors_or_nonzero() {
    // On a Linux box without the named machine, systemd-run exits non-zero
    // and we surface that in the result table. On non-Linux, the function
    // errors at runtime. Either is acceptable.
    let vm = create_vm();
    let result: mlua::Result<mlua::Table> = vm
        .load(r#"return systemd.machine_exec("does-not-exist-xyz", "/bin/true")"#)
        .eval_async()
        .await;
    if let Ok(t) = result {
        let status: i64 = t.get("status").unwrap();
        assert_ne!(status, 0, "expected non-zero status for missing machine");
    } else {
        // Non-Linux path: hard error is fine.
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            msg.contains("machine_exec") || msg.contains("Linux only"),
            "unexpected error: {msg}"
        );
    }
}
