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
async fn test_yaml_parse_all_documents() {
    let script = r#"
        local docs = yaml.parse_all("kind: Service\nmetadata:\n  name: api\n---\nkind: Deployment\nmetadata:\n  name: worker\n")
        assert.eq(#docs, 2)
        assert.eq(docs[1].kind, "Service")
        assert.eq(docs[1].metadata.name, "api")
        assert.eq(docs[2].kind, "Deployment")
        assert.eq(docs[2].metadata.name, "worker")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_parse_all_skips_empty_documents() {
    let script = r#"
        local docs = yaml.parse_all("---\nkind: ConfigMap\n---\n---\nkind: Secret\n")
        assert.eq(#docs, 2)
        assert.eq(docs[1].kind, "ConfigMap")
        assert.eq(docs[2].kind, "Secret")
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

#[tokio::test]
async fn test_yaml_parse_local_tag_on_scalar() {
    let script = r#"
        local data = yaml.parse("a: !Ref some-value\n")
        assert.eq(data.a["!Ref"], "some-value")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_parse_local_tag_on_sequence() {
    let script = r#"
        local data = yaml.parse(".anchor:\n  script:\n    - echo hi\njob:\n  script:\n    - !reference [.anchor, script]\n")
        assert.eq(data[".anchor"].script[1], "echo hi")
        local ref = data.job.script[1]["!reference"]
        assert.eq(#ref, 2)
        assert.eq(ref[1], ".anchor")
        assert.eq(ref[2], "script")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_parse_local_tag_on_mapping() {
    let script = r#"
        local data = yaml.parse("b: !custom {x: 1, y: two}\n")
        assert.eq(data.b["!custom"].x, 1)
        assert.eq(data.b["!custom"].y, "two")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_parse_standard_tags_resolve_to_scalars() {
    let script = r#"
        local data = yaml.parse("s: !!str 1\ni: !!int \"5\"\nf: !!float \"1.5\"\nb: !!bool \"true\"\nn: !!null \"~\"\nt: !!timestamp 2001-12-14\nlong: !<tag:yaml.org,2002:str> 1\n")
        assert.eq(type(data.s), "string")
        assert.eq(data.s, "1")
        assert.eq(data.i, 5)
        assert.eq(data.f, 1.5)
        assert.eq(data.b, true)
        assert.eq(data.n, nil)
        assert.eq(data.t, "2001-12-14")
        assert.eq(data.long, "1")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_parse_tag_inside_sequence() {
    let script = r#"
        local data = yaml.parse("items:\n  - !ref one\n  - plain\n  - !custom {k: v}\n")
        assert.eq(#data.items, 3)
        assert.eq(data.items[1]["!ref"], "one")
        assert.eq(data.items[2], "plain")
        assert.eq(data.items[3]["!custom"].k, "v")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_parse_tags_strip() {
    let script = r#"
        local data = yaml.parse("a: !custom {x: 1}\nb: !reference [.anchor, script]\nc: !Ref scalar\n", { tags = "strip" })
        assert.eq(data.a.x, 1)
        assert.eq(data.b[1], ".anchor")
        assert.eq(data.b[2], "script")
        assert.eq(data.c, "scalar")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_parse_tags_wrap_is_the_default() {
    let script = r#"
        local implicit = yaml.parse("a: !custom {x: 1}\n")
        local explicit = yaml.parse("a: !custom {x: 1}\n", { tags = "wrap" })
        assert.eq(implicit.a["!custom"].x, 1)
        assert.eq(explicit.a["!custom"].x, 1)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_parse_rejects_unknown_tags_option() {
    let script = r#"
        local ok, err = pcall(yaml.parse, "a: 1\n", { tags = "drop" })
        assert.eq(ok, false)
        assert.contains(tostring(err), "yaml.parse: tags must be \"wrap\" or \"strip\"")
        local ok2, err2 = pcall(yaml.parse_all, "a: 1\n", { tags = 7 })
        assert.eq(ok2, false)
        assert.contains(tostring(err2), "yaml.parse_all: tags must be \"wrap\" or \"strip\"")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_parse_all_with_tagged_document() {
    let script = r#"
        local docs = yaml.parse_all("kind: Pipeline\njob:\n  script:\n    - !reference [.anchor, script]\n---\n---\nkind: Plain\n")
        assert.eq(#docs, 2)
        assert.eq(docs[1].kind, "Pipeline")
        assert.eq(docs[1].job.script[1]["!reference"][2], "script")
        assert.eq(docs[2].kind, "Plain")

        local stripped = yaml.parse_all("a: !custom {x: 1}\n", { tags = "strip" })
        assert.eq(stripped[1].a.x, 1)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_encode_roundtrips_a_wrapped_tag() {
    let script = r#"
        local encoded = yaml.encode({ ["!reference"] = { ".anchor", "script" } })
        assert.eq(encoded, "!reference\n- '.anchor'\n- script\n")
        local back = yaml.parse(encoded)
        assert.eq(back["!reference"][1], ".anchor")
        assert.eq(back["!reference"][2], "script")

        local nested = yaml.parse("job:\n  script:\n    - !reference [.anchor, script]\n")
        local again = yaml.parse(yaml.encode(nested))
        assert.eq(again.job.script[1]["!reference"][1], ".anchor")
        assert.eq(again.job.script[1]["!reference"][2], "script")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_encode_untagged_keeps_sorted_keys() {
    let script = r#"
        assert.eq(yaml.encode({ z = 1, a = 2, m = { b = 1, a = 2 } }), "a: 2\nm:\n  a: 2\n  b: 1\nz: 1\n")
        assert.eq(yaml.encode({ "one", "two" }), "- one\n- two\n")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_parse_untagged_document_is_unchanged() {
    let script = r#"
        local data = yaml.parse("name: assay\nversion: 1\nratio: 1.5\nenabled: true\nempty: null\nlist:\n  - one\n  - two\nnested:\n  host: localhost\n  port: 8080\ntext: |\n  line one\n  line two\n")
        assert.eq(data.name, "assay")
        assert.eq(data.version, 1)
        assert.eq(data.ratio, 1.5)
        assert.eq(data.enabled, true)
        assert.eq(data.empty, nil)
        assert.eq(#data.list, 2)
        assert.eq(data.list[1], "one")
        assert.eq(data.nested.host, "localhost")
        assert.eq(data.nested.port, 8080)
        assert.contains(data.text, "line one")
        assert.contains(data.text, "line two")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_yaml_parse_rejects_non_string_mapping_key() {
    let script = r#"
        local ok, err = pcall(yaml.parse, "1: a\n")
        assert.eq(ok, false)
        assert.contains(tostring(err), "yaml.parse: unsupported YAML mapping key")
    "#;
    run_lua(script).await.unwrap();
}
