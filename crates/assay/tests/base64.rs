mod common;

use common::{eval_lua, run_lua};

#[tokio::test]
async fn test_base64_encode() {
    let result: String = eval_lua(r#"return base64.encode("hello world")"#).await;
    assert_eq!(result, "aGVsbG8gd29ybGQ=");
}

#[tokio::test]
async fn test_base64_decode() {
    let result: String = eval_lua(r#"return base64.decode("aGVsbG8gd29ybGQ=")"#).await;
    assert_eq!(result, "hello world");
}

#[tokio::test]
async fn test_base64_roundtrip() {
    let script = r#"
        local original = "special chars: !@#$%^&*()_+-={}[]|;':\",./<>?"
        local encoded = base64.encode(original)
        local decoded = base64.decode(encoded)
        assert.eq(decoded, original)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_base64_empty() {
    let result: String = eval_lua(r#"return base64.encode("")"#).await;
    assert_eq!(result, "");
}
