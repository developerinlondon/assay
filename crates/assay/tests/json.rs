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

// Empty Lua tables encode to JSON objects (`{}`), not arrays (`[]`).
// Regression test for issue #129 — every assay http builtin caller that
// passed `{}` as a request body was silently shipping `[]`.
#[tokio::test]
async fn test_empty_table_encodes_as_object() {
    let script = r#"
        assert.eq(json.encode({}), "{}")
    "#;
    run_lua(script).await.unwrap();
}

// json.array(t) escape hatch — pin a table to JSON array shape via the
// __jsontype metatable marker, regardless of whether it's empty.
#[tokio::test]
async fn test_json_array_helper_pins_array_shape() {
    let script = r#"
        assert.eq(json.encode(json.array({})), "[]")
        assert.eq(json.encode(json.array()), "[]")
        assert.eq(json.encode(json.array({1, 2, 3})), "[1,2,3]")
    "#;
    run_lua(script).await.unwrap();
}

// json.object(t) escape hatch — force object shape on a table whose keys
// happen to be 1..N integers.
#[tokio::test]
async fn test_json_object_helper_pins_object_shape() {
    let script = r#"
        local t = json.object({ "a", "b", "c" })
        local s = json.encode(t)
        -- Object key order is not guaranteed by serde_json; check the
        -- shape characters and content separately.
        assert.eq(s:sub(1, 1), "{")
        assert.eq(s:sub(-1), "}")
        assert.contains(s, '"1":"a"')
        assert.contains(s, '"2":"b"')
        assert.contains(s, '"3":"c"')
    "#;
    run_lua(script).await.unwrap();
}

// Sequential 1..N tables still encode as arrays (existing behaviour).
#[tokio::test]
async fn test_sequential_table_still_array() {
    let script = r#"
        assert.eq(json.encode({10, 20, 30}), "[10,20,30]")
    "#;
    run_lua(script).await.unwrap();
}

// String-keyed tables encode as objects.
#[tokio::test]
async fn test_string_keyed_table_encodes_as_object() {
    let script = r#"
        local s = json.encode({ name = "assay" })
        assert.eq(s, '{"name":"assay"}')
    "#;
    run_lua(script).await.unwrap();
}

// Nested empties: an object with an empty-table field encodes the field
// as `{}`, mirroring the top-level rule.
#[tokio::test]
async fn test_nested_empty_table_encodes_as_object() {
    let script = r#"
        assert.eq(json.encode({ inner = {} }), '{"inner":{}}')
    "#;
    run_lua(script).await.unwrap();
}

// Mixed nesting: a top-level object with an explicit empty array field.
#[tokio::test]
async fn test_nested_explicit_empty_array() {
    let script = r#"
        assert.eq(json.encode({ items = json.array({}) }), '{"items":[]}')
    "#;
    run_lua(script).await.unwrap();
}

// json.array preserves an existing metatable. A table with __index set
// for inheritance keeps that hook after json.array tags it.
#[tokio::test]
async fn test_json_array_preserves_existing_metatable() {
    let script = r#"
        local parent = { greeting = "hi" }
        local t = setmetatable({}, { __index = parent })
        json.array(t)
        assert.eq(t.greeting, "hi")              -- __index still works
        assert.eq(json.encode(t), "[]")          -- __jsontype applied
    "#;
    run_lua(script).await.unwrap();
}
