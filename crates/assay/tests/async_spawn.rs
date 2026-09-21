mod common;

use common::run_lua_local;

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
    let script = r#"
        local function measure(body)
            local start = time()
            body()
            return time() - start
        end

        local sequential = measure(function()
            async.spawn(function() sleep(0.2) end).await()
            async.spawn(function() sleep(0.2) end).await()
        end)

        local concurrent = measure(function()
            local h1 = async.spawn(function() sleep(0.2) end)
            local h2 = async.spawn(function() sleep(0.2) end)
            h1.await()
            h2.await()
        end)

        assert.lt(
            concurrent,
            sequential * 0.75,
            string.format(
                "concurrent spawns should overlap: sequential %.3fs, concurrent %.3fs",
                sequential,
                concurrent
            )
        )
    "#;
    run_lua_local(script).await.unwrap();
}

#[tokio::test]
async fn test_async_spawn_interval_basic() {
    let script = r#"
        local count = 0
        local handle = async.spawn_interval(0.02, function()
            count = count + 1
        end)

        local deadline = time() + 5
        while count < 2 and time() < deadline do
            sleep(0.01)
        end
        handle.cancel()

        assert.gt(count, 1, string.format("spawn_interval fired %d times in 5s", count))
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

        local start = time()
        local deadline = start + 5
        while count < 2 and time() < deadline do
            sleep(0.01)
        end
        assert.gt(count, 1, string.format("spawn_interval fired %d times in 5s", count))

        -- Wait out several ticks at the rate this host actually delivered them,
        -- so a slow scheduler lengthens the observation window too.
        local ticked_in = time() - start
        handle.cancel()
        local count_at_cancel = count
        sleep(ticked_in * 2)

        assert.eq(
            count,
            count_at_cancel,
            string.format("interval fired %d more times after cancel", count - count_at_cancel)
        )
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
