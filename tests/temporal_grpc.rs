#![cfg(feature = "temporal")]

mod common;

use common::{eval_lua, run_lua};

#[tokio::test]
async fn test_temporal_table_registered() {
    let result: bool = eval_lua(
        r#"
        return type(temporal) == "table"
        "#,
    )
    .await;
    assert!(result);
}

#[tokio::test]
async fn test_temporal_connect_is_function() {
    let result: bool = eval_lua(
        r#"
        return type(temporal.connect) == "function"
        "#,
    )
    .await;
    assert!(result);
}

#[tokio::test]
async fn test_temporal_start_is_function() {
    let result: bool = eval_lua(
        r#"
        return type(temporal.start) == "function"
        "#,
    )
    .await;
    assert!(result);
}

#[tokio::test]
async fn test_temporal_connect_invalid_url() {
    let result = run_lua(r#"temporal.connect({ url = "://bad url" })"#).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("invalid temporal URL"),
        "expected 'invalid temporal URL' in: {err}"
    );
}

#[tokio::test]
async fn test_temporal_connect_unreachable() {
    let result = run_lua(r#"temporal.connect({ url = "127.0.0.1:19999" })"#).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("temporal connect"),
        "expected 'temporal connect' in: {err}"
    );
}

#[tokio::test]
async fn test_temporal_start_missing_url() {
    let result = run_lua(
        r#"
        temporal.start({
            task_queue = "q",
            workflow_type = "t",
            workflow_id = "id",
        })
        "#,
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_temporal_start_missing_task_queue() {
    let result = run_lua(
        r#"
        temporal.start({
            url = "127.0.0.1:19999",
            workflow_type = "t",
            workflow_id = "id",
        })
        "#,
    )
    .await;
    // Should fail — either missing field or connection error
    assert!(result.is_err());
}

#[tokio::test]
async fn test_temporal_stdlib_still_works() {
    let result: bool = eval_lua(
        r#"
        local t = require("assay.temporal")
        return type(t.client) == "function"
        "#,
    )
    .await;
    assert!(result);
}
