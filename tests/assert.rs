mod common;

use common::run_lua;

#[tokio::test]
async fn test_assert_eq_pass() {
    run_lua(r#"assert.eq(42, 42)"#).await.unwrap();
}

#[tokio::test]
async fn test_assert_eq_fail() {
    let result = run_lua(r#"assert.eq(1, 2, "numbers differ")"#).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("assert.eq failed"), "got: {err}");
    assert!(err.contains("numbers differ"), "got: {err}");
}

#[tokio::test]
async fn test_assert_gt() {
    run_lua(r#"assert.gt(10, 5)"#).await.unwrap();
    assert!(run_lua(r#"assert.gt(5, 10)"#).await.is_err());
}

#[tokio::test]
async fn test_assert_lt() {
    run_lua(r#"assert.lt(5, 10)"#).await.unwrap();
    assert!(run_lua(r#"assert.lt(10, 5)"#).await.is_err());
}

#[tokio::test]
async fn test_assert_contains() {
    run_lua(r#"assert.contains("hello world", "world")"#)
        .await
        .unwrap();
    assert!(run_lua(r#"assert.contains("hello", "xyz")"#).await.is_err());
}

#[tokio::test]
async fn test_assert_not_nil() {
    run_lua(r#"assert.not_nil("something")"#).await.unwrap();
    assert!(run_lua(r#"assert.not_nil(nil)"#).await.is_err());
}
