mod common;

use common::run_lua;

#[tokio::test]
async fn test_shell_exec_basic() {
    let script = r#"
        local result = shell.exec("echo hello")
        assert.eq(result.status, 0)
        assert.contains(result.stdout, "hello")
        assert.eq(result.timed_out, false)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_shell_exec_stderr() {
    let script = r#"
        local result = shell.exec("echo error >&2")
        assert.eq(result.status, 0)
        assert.contains(result.stderr, "error")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_shell_exec_exit_code() {
    let script = r#"
        local result = shell.exec("exit 42")
        assert.eq(result.status, 42)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_shell_exec_with_cwd() {
    let script = r#"
        local result = shell.exec("pwd", { cwd = "/tmp" })
        assert.eq(result.status, 0)
        assert.contains(result.stdout, "/tmp")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_shell_exec_with_env() {
    let script = r#"
        local result = shell.exec("echo $MY_VAR", { env = { MY_VAR = "test_value" } })
        assert.eq(result.status, 0)
        assert.contains(result.stdout, "test_value")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_shell_exec_with_stdin() {
    let script = r#"
        local result = shell.exec("cat", { stdin = "hello from stdin" })
        assert.eq(result.status, 0)
        assert.contains(result.stdout, "hello from stdin")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_shell_exec_with_timeout() {
    let script = r#"
        local result = shell.exec("sleep 10", { timeout = 0.2 })
        assert.eq(result.timed_out, true)
        assert.eq(result.status, -1)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_shell_exec_multiline_output() {
    let script = r#"
        local result = shell.exec("echo line1; echo line2; echo line3")
        assert.eq(result.status, 0)
        assert.contains(result.stdout, "line1")
        assert.contains(result.stdout, "line2")
        assert.contains(result.stdout, "line3")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_shell_exec_pipe() {
    let script = r#"
        local result = shell.exec("echo 'hello world' | tr 'h' 'H'")
        assert.eq(result.status, 0)
        assert.contains(result.stdout, "Hello")
    "#;
    run_lua(script).await.unwrap();
}
