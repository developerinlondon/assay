mod common;

use common::{eval_lua, run_lua};

#[tokio::test]
async fn test_toml_parse_table() {
    let script = r#"
        local data = toml.parse("[package]\nname = \"assay\"\nversion = \"0.1.0\"\n")
        assert.eq(data.package.name, "assay")
        assert.eq(data.package.version, "0.1.0")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_toml_parse_nested() {
    let script = r#"
        local input = "[server]\nhost = \"localhost\"\nport = 8080\n\n[server.tls]\nenabled = true\n"
        local data = toml.parse(input)
        assert.eq(data.server.host, "localhost")
        assert.eq(data.server.port, 8080)
        assert.eq(data.server.tls.enabled, true)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_toml_parse_array() {
    let script = r#"
        local data = toml.parse("values = [1, 2, 3]\n")
        assert.eq(#data.values, 3)
        assert.eq(data.values[1], 1)
        assert.eq(data.values[3], 3)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_toml_encode_table() {
    let script = r#"
        local encoded = toml.encode({name = "assay", version = "0.1.0"})
        assert.contains(encoded, "name")
        assert.contains(encoded, "assay")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_toml_roundtrip() {
    let script = r#"
        local input = "greeting = \"hello\"\ncount = 42\n"
        local data = toml.parse(input)
        assert.eq(data.greeting, "hello")
        assert.eq(data.count, 42)
        local encoded = toml.encode(data)
        local data2 = toml.parse(encoded)
        assert.eq(data2.greeting, "hello")
        assert.eq(data2.count, 42)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_toml_parse_error() {
    let result = run_lua(r#"toml.parse("= invalid")"#).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_toml_types() {
    let script = r#"
        local input = "str = \"hello\"\nnum = 42\nfloat = 3.14\nbool = true\n"
        local data = toml.parse(input)
        assert.eq(data.str, "hello")
        assert.eq(data.num, 42)
        assert.eq(data.bool, true)
        assert.gt(data.float, 3.13)
        assert.lt(data.float, 3.15)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_toml_encode_then_parse_preserves_types() {
    let result: bool = eval_lua(
        r#"
        local t = {name = "test", count = 10, active = true}
        local encoded = toml.encode(t)
        local decoded = toml.parse(encoded)
        return decoded.count == 10 and decoded.active == true and decoded.name == "test"
    "#,
    )
    .await;
    assert!(result);
}
