mod common;

use common::run_lua;

#[tokio::test]
async fn test_json_parse_and_encode() {
    let script = r#"
        local data = json.parse('{"name":"assay","version":1}')
        assert.eq(data.name, "assay")
        assert.eq(data.version, 1)
        local encoded = json.encode(data)
        assert.contains(encoded, '"name"')
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_json_array() {
    let script = r#"
        local arr = json.parse('[1,2,3]')
        assert.eq(#arr, 3)
        assert.eq(arr[1], 1)
        assert.eq(arr[3], 3)
    "#;
    run_lua(script).await.unwrap();
}
