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
