mod common;

use common::{eval_lua, run_lua};

#[tokio::test]
async fn test_match_true() {
    let result: bool = eval_lua(r#"return regex.match("hello world", "^hello")"#).await;
    assert!(result);
}

#[tokio::test]
async fn test_match_false() {
    let result: bool = eval_lua(r#"return regex.match("hello world", "^world")"#).await;
    assert!(!result);
}

#[tokio::test]
async fn test_find_with_groups() {
    let script = r#"
        local result = regex.find("2026-02-10", "^(\\d{4})-(\\d{2})-(\\d{2})$")
        assert.eq(result.match, "2026-02-10")
        assert.eq(result.groups[1], "2026")
        assert.eq(result.groups[2], "02")
        assert.eq(result.groups[3], "10")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_find_no_match() {
    let script = r#"
        local result = regex.find("hello", "^\\d+$")
        assert.eq(result, nil)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_find_all() {
    let script = r#"
        local results = regex.find_all("foo123bar456baz", "\\d+")
        assert.eq(#results, 2)
        assert.eq(results[1], "123")
        assert.eq(results[2], "456")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_replace() {
    let result: String =
        eval_lua(r#"return regex.replace("hello world", "world", "lua")"#).await;
    assert_eq!(result, "hello lua");
}

#[tokio::test]
async fn test_replace_all() {
    let result: String =
        eval_lua(r#"return regex.replace("aaa bbb aaa", "aaa", "ccc")"#).await;
    assert_eq!(result, "ccc bbb ccc");
}

#[tokio::test]
async fn test_replace_with_capture_groups() {
    let result: String =
        eval_lua(r#"return regex.replace("John Smith", "(\\w+) (\\w+)", "$2, $1")"#).await;
    assert_eq!(result, "Smith, John");
}

#[tokio::test]
async fn test_invalid_pattern() {
    assert!(run_lua(r#"regex.match("test", "[")"#).await.is_err());
}
