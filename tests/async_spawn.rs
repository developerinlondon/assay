mod common;

use common::{eval_lua_local, run_lua_local};

#[tokio::test]
async fn test_async_spawn_basic() {
    let script = r#"
        local handle = async.spawn(function()
            return 42
        end)
        local result = handle.await()
        assert.eq(result[1], 42)
    "#;
    run_lua_local(script).await.unwrap();
}

#[tokio::test]
async fn test_async_spawn_with_sleep() {
    let script = r#"
        local handle = async.spawn(function()
            sleep(0.01)
            return "done"
        end)
        local result = handle.await()
        assert.eq(result[1], "done")
    "#;
    run_lua_local(script).await.unwrap();
}

#[tokio::test]
async fn test_async_spawn_multiple() {
    let script = r#"
        local h1 = async.spawn(function() return 1 end)
        local h2 = async.spawn(function() return 2 end)
        local h3 = async.spawn(function() return 3 end)
        local r1 = h1.await()
        local r2 = h2.await()
        local r3 = h3.await()
        assert.eq(r1[1], 1)
        assert.eq(r2[1], 2)
        assert.eq(r3[1], 3)
    "#;
    run_lua_local(script).await.unwrap();
}

#[tokio::test]
async fn test_async_spawn_error_propagation() {
    let result = run_lua_local(
        r#"
        local handle = async.spawn(function()
            error("task failed")
        end)
        handle.await()
    "#,
    )
    .await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("task failed"), "got: {err_msg}");
}

#[tokio::test]
async fn test_async_spawn_double_await_errors() {
    let result = run_lua_local(
        r#"
        local handle = async.spawn(function() return 1 end)
        handle.await()
        handle.await()
    "#,
    )
    .await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("already awaited"), "got: {err_msg}");
}

#[tokio::test]
async fn test_async_spawn_concurrent_execution() {
    let result: bool = eval_lua_local(
        r#"
        local start = time()
        local h1 = async.spawn(function() sleep(0.05) end)
        local h2 = async.spawn(function() sleep(0.05) end)
        h1.await()
        h2.await()
        local elapsed = time() - start
        return elapsed < 0.15
    "#,
    )
    .await;
    assert!(result, "concurrent spawns should run in parallel");
}

#[tokio::test]
async fn test_async_spawn_interval_basic() {
    let script = r#"
        local count = 0
        local handle = async.spawn_interval(0.02, function()
            count = count + 1
        end)
        sleep(0.09)
        handle.cancel()
        assert.gt(count, 1)
    "#;
    run_lua_local(script).await.unwrap();
}

#[tokio::test]
async fn test_async_spawn_interval_cancel() {
    let script = r#"
        local count = 0
        local handle = async.spawn_interval(0.02, function()
            count = count + 1
        end)
        sleep(0.05)
        handle.cancel()
        local count_at_cancel = count
        sleep(0.05)
        assert.eq(count, count_at_cancel)
    "#;
    run_lua_local(script).await.unwrap();
}

#[tokio::test]
async fn test_async_spawn_interval_invalid_seconds() {
    let result = run_lua_local(r#"async.spawn_interval(0, function() end)"#).await;
    assert!(result.is_err());
    let result2 = run_lua_local(r#"async.spawn_interval(-1, function() end)"#).await;
    assert!(result2.is_err());
}
