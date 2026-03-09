mod common;

use common::run_lua;

#[tokio::test]
async fn test_process_list() {
    let script = r#"
        local procs = process.list()
        assert.not_nil(procs)
        -- There should be at least one process (the test runner itself)
        local count = 0
        for _ in ipairs(procs) do count = count + 1 end
        assert.gt(count, 0, "expected at least one process")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_process_list_has_pid_and_name() {
    let script = r#"
        local procs = process.list()
        local found = false
        for _, p in ipairs(procs) do
            assert.not_nil(p.pid)
            assert.not_nil(p.name)
            found = true
            break
        end
        assert.eq(found, true, "expected to find at least one process with pid and name")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_process_is_running_known() {
    // Check that is_running returns false for a definitely-nonexistent process
    let script = r#"
        local result = process.is_running("definitely_not_a_real_process_name_xyz")
        assert.eq(result, false)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_process_kill_nonexistent() {
    // Trying to kill a non-existent PID should error
    let result = run_lua(r#"process.kill(999999999)"#).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_process_kill_signal_zero() {
    // Signal 0 checks if process exists without killing it.
    // Use our own spawned process instead of PID 1 (which requires root on CI).
    let script = r#"
        local result = shell.exec("sleep 300 </dev/null >/dev/null 2>&1 & echo $!")
        assert.eq(result.status, 0)
        local pid = tonumber(result.stdout:match("(%d+)"))
        assert.not_nil(pid)

        -- Signal 0 should succeed for our own child process
        process.kill(pid, 0)

        -- Clean up
        process.kill(pid, 9)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_process_kill_spawned() {
    // Spawn a background process via shell, then kill it
    // Note: Must redirect bg process stdout/stderr or shell.exec hangs waiting for pipe close
    let script = r#"
        local result = shell.exec("sleep 300 </dev/null >/dev/null 2>&1 & echo $!")
        assert.eq(result.status, 0)
        local pid = tonumber(result.stdout:match("(%d+)"))
        assert.not_nil(pid)

        -- Verify it's running (signal 0 = check existence)
        process.kill(pid, 0)

        -- Kill it with SIGKILL for immediate termination
        process.kill(pid, 9)

        -- Give kernel time to reap
        sleep(0.5)

        -- Verify it's gone (kill with signal 0 should fail)
        local ok = pcall(process.kill, pid, 0)
        assert.eq(ok, false)
    "#;
    run_lua(script).await.unwrap();
}
