mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_assay_prometheus() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": {
                "resultType": "vector",
                "result": [{
                    "metric": {"__name__": "up"},
                    "value": [1700000000.0, "42"]
                }]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local prom = require("assay.prometheus")
        local val = prom.query("{}", "up")
        assert.eq(val, 42)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_require_nonexistent_module() {
    let result = run_lua(r#"require("assay.nonexistent")"#).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_require_openbao() {
    let script = r#"
        local bao = require("assay.openbao")
        assert.not_nil(bao)
        assert.not_nil(bao.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_openbao_read() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/mykey"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {"data": {"username": "admin", "password": "secret123"}}
            })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local bao = require("assay.openbao")
        local c = bao.client("{}", "test-token")
        local data = c:read("secret/data/mykey")
        assert.eq(data.data.username, "admin")
        assert.eq(data.data.password, "secret123")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_openbao_kv_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/mykey"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {"data": {"foo": "bar"}}
            })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local bao = require("assay.openbao")
        local c = bao.client("{}", "test-token")
        local data = c:kv_get("secret", "mykey")
        assert.eq(data.data.foo, "bar")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_openbao_write() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/secret/data/newkey"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local bao = require("assay.openbao")
        local c = bao.client("{}", "test-token")
        c:write("secret/data/newkey", {{ data = {{ key = "value" }} }})
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_openbao_read_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local bao = require("assay.openbao")
        local c = bao.client("{}", "test-token")
        local data = c:read("secret/data/missing")
        assert.eq(data, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
