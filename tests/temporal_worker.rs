#![cfg(feature = "temporal")]

mod common;

use common::{eval_lua, run_lua};

#[tokio::test]
async fn test_temporal_worker_is_function() {
    let result: bool = eval_lua(
        r#"
        return type(temporal.worker) == "function"
        "#,
    )
    .await;
    assert!(result);
}

#[tokio::test]
async fn test_temporal_worker_requires_activities_or_workflows() {
    let result = run_lua(
        r#"
        temporal.worker({
            url = "127.0.0.1:19999",
            task_queue = "test-queue",
        })
        "#,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("at least one activity or workflow"),
        "expected 'at least one activity or workflow' in: {err}"
    );
}

#[tokio::test]
async fn test_temporal_worker_requires_url() {
    let result = run_lua(
        r#"
        temporal.worker({
            task_queue = "test-queue",
            activities = {
                echo = function(input) return input end,
            },
        })
        "#,
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_temporal_worker_requires_task_queue() {
    let result = run_lua(
        r#"
        temporal.worker({
            url = "127.0.0.1:19999",
            activities = {
                echo = function(input) return input end,
            },
        })
        "#,
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_temporal_worker_invalid_url() {
    let result = run_lua(
        r#"
        temporal.worker({
            url = "://bad-url",
            task_queue = "test-queue",
            activities = {
                echo = function(input) return input end,
            },
        })
        "#,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("invalid temporal URL"),
        "expected 'invalid temporal URL' in: {err}"
    );
}

#[tokio::test]
async fn test_temporal_worker_unreachable_server() {
    let result = run_lua(
        r#"
        temporal.worker({
            url = "127.0.0.1:19999",
            task_queue = "test-queue",
            activities = {
                echo = function(input) return input end,
            },
        })
        "#,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("temporal worker connect") || err.contains("temporal worker init"),
        "expected connection error in: {err}"
    );
}

#[tokio::test]
async fn test_temporal_worker_accepts_workflows_only() {
    // Workflows-only (no activities) should still require a valid server,
    // but the registration itself should not error on missing activities.
    let result = run_lua(
        r#"
        temporal.worker({
            url = "127.0.0.1:19999",
            task_queue = "test-queue",
            workflows = {
                MyWorkflow = function(ctx, input) return { status = "done" } end,
            },
        })
        "#,
    )
    .await;
    // Should fail on connection, not on registration
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("temporal worker connect") || err.contains("temporal worker init"),
        "expected connection error (not registration error) in: {err}"
    );
}

#[tokio::test]
async fn test_temporal_worker_accepts_both_activities_and_workflows() {
    let result = run_lua(
        r#"
        temporal.worker({
            url = "127.0.0.1:19999",
            task_queue = "test-queue",
            activities = {
                echo = function(input) return input end,
            },
            workflows = {
                MyWorkflow = function(ctx, input) return { status = "done" } end,
            },
        })
        "#,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("temporal worker connect") || err.contains("temporal worker init"),
        "expected connection error in: {err}"
    );
}

#[tokio::test]
async fn test_temporal_worker_ctx_side_effect_and_info() {
    // Test the ctx pattern used by the workflow bridge: side_effect and workflow_info
    let result: bool = eval_lua(
        r#"
        local ctx = {
            _next_seq = 1,
            _resolved = {},
            _signals = {},
            _info = { workflow_id = "test-1", workflow_type = "Test" },
        }
        function ctx:side_effect(fn)
            local seq = self._next_seq
            self._next_seq = seq + 1
            return fn()
        end
        function ctx:workflow_info()
            return self._info
        end

        local id = ctx:side_effect(function() return "abc123" end)
        local info = ctx:workflow_info()
        return id == "abc123"
            and info.workflow_id == "test-1"
            and info.workflow_type == "Test"
            and ctx._next_seq == 2
        "#,
    )
    .await;
    assert!(result);
}
