mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_http_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local hc = require("assay.healthcheck")
        local result = hc.http("{}/health")
        assert.eq(result.ok, true)
        assert.eq(result.status, 200)
        assert.eq(result.error, nil)
        assert.not_nil(result.latency_ms)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_http_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local hc = require("assay.healthcheck")
        local result = hc.http("{}/health")
        assert.eq(result.ok, false)
        assert.eq(result.status, 500)
        assert.not_nil(result.error)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_http_custom_expected_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/created"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local hc = require("assay.healthcheck")
        local result = hc.http("{}/created", {{ expected_status = 201 }})
        assert.eq(result.ok, true)
        assert.eq(result.status, 201)
        assert.eq(result.error, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_json_path_matching() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": {
                "ready": true,
                "version": "1.2.3"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local hc = require("assay.healthcheck")
        local result = hc.json_path("{}/api/status", "status.ready", true)
        assert.eq(result.ok, true)
        assert.eq(result.actual, true)
        assert.eq(result.expected, true)
        assert.eq(result.error, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_json_path_non_matching() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": {
                "ready": false
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local hc = require("assay.healthcheck")
        local result = hc.json_path("{}/api/status", "status.ready", true)
        assert.eq(result.ok, false)
        assert.eq(result.actual, false)
        assert.eq(result.expected, true)
        assert.not_nil(result.error)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_json_path_missing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": {}})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local hc = require("assay.healthcheck")
        local result = hc.json_path("{}/api/status", "status.ready", true)
        assert.eq(result.ok, false)
        assert.eq(result.actual, nil)
        assert.not_nil(result.error)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_status_code_match() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ping"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local hc = require("assay.healthcheck")
        local result = hc.status_code("{}/ping", 204)
        assert.eq(result.ok, true)
        assert.eq(result.status, 204)
        assert.eq(result.error, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_status_code_mismatch() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ping"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local hc = require("assay.healthcheck")
        local result = hc.status_code("{}/ping", 200)
        assert.eq(result.ok, false)
        assert.eq(result.status, 503)
        assert.not_nil(result.error)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_body_contains_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/info"))
        .respond_with(ResponseTemplate::new(200).set_body_string("server is healthy and running"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local hc = require("assay.healthcheck")
        local result = hc.body_contains("{}/info", "healthy")
        assert.eq(result.ok, true)
        assert.eq(result.found, true)
        assert.eq(result.error, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_body_contains_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/info"))
        .respond_with(ResponseTemplate::new(200).set_body_string("server is down"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local hc = require("assay.healthcheck")
        local result = hc.body_contains("{}/info", "healthy")
        assert.eq(result.ok, false)
        assert.eq(result.found, false)
        assert.not_nil(result.error)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_endpoint_healthy() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ready"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local hc = require("assay.healthcheck")
        local result = hc.endpoint("{}/ready")
        assert.eq(result.ok, true)
        assert.eq(result.status, 200)
        assert.not_nil(result.latency_ms)
        assert.eq(result.error, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_endpoint_unhealthy_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ready"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local hc = require("assay.healthcheck")
        local result = hc.endpoint("{}/ready")
        assert.eq(result.ok, false)
        assert.eq(result.status, 503)
        assert.not_nil(result.error)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_multi_all_pass() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/svc1/health"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/svc2/health"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local hc = require("assay.healthcheck")
        local base = "{}"
        local result = hc.multi({{{{
            name = "service-1",
            check = function() return hc.http(base .. "/svc1/health") end,
        }}, {{
            name = "service-2",
            check = function() return hc.http(base .. "/svc2/health") end,
        }}}})
        assert.eq(result.ok, true)
        assert.eq(result.passed, 2)
        assert.eq(result.failed, 0)
        assert.eq(result.total, 2)
        assert.eq(result.results[1].name, "service-1")
        assert.eq(result.results[1].ok, true)
        assert.eq(result.results[2].name, "service-2")
        assert.eq(result.results[2].ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_multi_some_fail() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/svc1/health"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/svc2/health"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local hc = require("assay.healthcheck")
        local base = "{}"
        local result = hc.multi({{{{
            name = "service-1",
            check = function() return hc.http(base .. "/svc1/health") end,
        }}, {{
            name = "service-2",
            check = function() return hc.http(base .. "/svc2/health") end,
        }}}})
        assert.eq(result.ok, false)
        assert.eq(result.passed, 1)
        assert.eq(result.failed, 1)
        assert.eq(result.total, 2)
        assert.eq(result.results[1].ok, true)
        assert.eq(result.results[2].ok, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
