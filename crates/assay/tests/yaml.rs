mod common;

use common::{eval_lua, run_lua};

#[tokio::test]
async fn test_yaml_parse_object() {
    let script = r#"
        local data = yaml.parse("name: assay\nversion: 1\n")
        assert.eq(data.name, "assay")
        assert.eq(data.version, 1)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_parse_array() {
    let script = r#"
        local data = yaml.parse("- one\n- two\n- three\n")
        assert.eq(#data, 3)
        assert.eq(data[1], "one")
        assert.eq(data[3], "three")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_parse_nested() {
    let script = r#"
        local data = yaml.parse("server:\n  host: localhost\n  port: 8080\n")
        assert.eq(data.server.host, "localhost")
        assert.eq(data.server.port, 8080)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_encode_table() {
    let script = r#"
        local encoded = yaml.encode({name = "assay", version = 1})
        assert.contains(encoded, "name")
        assert.contains(encoded, "assay")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_roundtrip() {
    let script = r#"
        local input = "greeting: hello\ncount: 42\n"
        local data = yaml.parse(input)
        assert.eq(data.greeting, "hello")
        assert.eq(data.count, 42)
        local encoded = yaml.encode(data)
        local data2 = yaml.parse(encoded)
        assert.eq(data2.greeting, "hello")
        assert.eq(data2.count, 42)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_parse_error() {
    let result = run_lua(r#"yaml.parse(":\n  :\n  - ][")"#).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_yaml_parse_multiline_string() {
    let script = r#"
        local data = yaml.parse("text: |\n  line one\n  line two\n")
        assert.contains(data.text, "line one")
        assert.contains(data.text, "line two")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_parse_boolean_and_null() {
    let script = r#"
        local data = yaml.parse("enabled: true\ndisabled: false\nempty: null\n")
        assert.eq(data.enabled, true)
        assert.eq(data.disabled, false)
        assert.eq(data.empty, nil)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_encode_then_parse_preserves_types() {
    let result: bool = eval_lua(
        r#"
        local t = {name = "test", count = 10, active = true}
        local encoded = yaml.encode(t)
        local decoded = yaml.parse(encoded)
        return decoded.count == 10 and decoded.active == true and decoded.name == "test"
    "#,
    )
    .await;
    assert!(result);
}
