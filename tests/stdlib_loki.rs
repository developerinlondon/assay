mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_loki_client_ready_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ready"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ready"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local loki = require("assay.loki")
        local c = loki.client("{}")
        local ok = c:ready()
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_loki_client_ready_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ready"))
        .respond_with(ResponseTemplate::new(503).set_body_string("not ready"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local loki = require("assay.loki")
        local c = loki.client("{}")
        local ok = c:ready()
        assert.eq(ok, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_loki_push_string_entries() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/loki/api/v1/push"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local loki = require("assay.loki")
        local c = loki.client("{}")
        local ok = c:push(
            {{app = "myapp", env = "test"}},
            {{"log line one", "log line two", "log line three"}}
        )
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_loki_push_timestamp_entries() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/loki/api/v1/push"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local loki = require("assay.loki")
        local c = loki.client("{}")
        local ok = c:push(
            {{app = "myapp"}},
            {{{{1234567890000000000, "first line"}}, {{1234567891000000000, "second line"}}}}
        )
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_loki_query() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/loki/api/v1/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": {
                "resultType": "streams",
                "result": [{
                    "stream": {"app": "myapp"},
                    "values": [
                        ["1234567890000000000", "log line one"],
                        ["1234567891000000000", "log line two"]
                    ]
                }]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local loki = require("assay.loki")
        local c = loki.client("{}")
        local result = c:query('{{app="myapp"}}')
        assert.eq(#result, 1)
        assert.eq(result[1].stream.app, "myapp")
        assert.eq(#result[1].values, 2)
        assert.eq(result[1].values[1][2], "log line one")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_loki_query_range() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/loki/api/v1/query_range"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": {
                "resultType": "streams",
                "result": [{
                    "stream": {"app": "myapp", "env": "test"},
                    "values": [
                        ["1234567890000000000", "line one"],
                        ["1234567891000000000", "line two"],
                        ["1234567892000000000", "line three"]
                    ]
                }]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local loki = require("assay.loki")
        local c = loki.client("{}")
        local result = c:query_range('{{app="myapp"}}', {{
            start = "1234567890",
            end_time = "1234567900",
            limit = "100",
            step = "5s",
        }})
        assert.eq(#result, 1)
        assert.eq(result[1].stream.env, "test")
        assert.eq(#result[1].values, 3)
        assert.eq(result[1].values[3][2], "line three")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_loki_labels() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/loki/api/v1/labels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": ["app", "env", "host", "level"]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local loki = require("assay.loki")
        local c = loki.client("{}")
        local labels = c:labels()
        assert.eq(#labels, 4)
        assert.eq(labels[1], "app")
        assert.eq(labels[4], "level")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_loki_label_values() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/loki/api/v1/label/app/values"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": ["frontend", "backend", "worker"]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local loki = require("assay.loki")
        local c = loki.client("{}")
        local values = c:label_values("app")
        assert.eq(#values, 3)
        assert.eq(values[1], "frontend")
        assert.eq(values[2], "backend")
        assert.eq(values[3], "worker")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_loki_series() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/loki/api/v1/series"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": [
                {"app": "frontend", "env": "prod"},
                {"app": "backend", "env": "prod"}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local loki = require("assay.loki")
        local c = loki.client("{}")
        local sel = loki.selector({{app = "frontend"}})
        local series = c:series({{sel}})
        assert.eq(#series, 2)
        assert.eq(series[1].app, "frontend")
        assert.eq(series[2].app, "backend")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_loki_tail() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/loki/api/v1/tail"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "streams": [{
                "stream": {"app": "myapp"},
                "values": [
                    ["1234567890000000000", "tail line one"],
                    ["1234567891000000000", "tail line two"]
                ]
            }],
            "dropped_entries": []
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local loki = require("assay.loki")
        local c = loki.client("{}")
        local data = c:tail('{{app="myapp"}}', {{limit = "10"}})
        assert.eq(#data.streams, 1)
        assert.eq(data.streams[1].stream.app, "myapp")
        assert.eq(#data.streams[1].values, 2)
        assert.eq(data.streams[1].values[1][2], "tail line one")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_loki_metrics() {
    let server = MockServer::start().await;
    let metrics_body = "# HELP loki_ingester_streams Total streams\n# TYPE loki_ingester_streams gauge\nloki_ingester_streams 42\n";
    Mock::given(method("GET"))
        .and(path("/metrics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(metrics_body))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local loki = require("assay.loki")
        local c = loki.client("{}")
        local body = c:metrics()
        assert.contains(body, "loki_ingester_streams 42")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_loki_selector() {
    let script = r#"
        local loki = require("assay.loki")
        local sel = loki.selector({app = "myapp"})
        assert.contains(sel, 'app="myapp"')
        assert.contains(sel, "{")
        assert.contains(sel, "}")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_loki_selector_multiple_labels() {
    let script = r#"
        local loki = require("assay.loki")
        local sel = loki.selector({app = "myapp", env = "test"})
        assert.contains(sel, 'app="myapp"')
        assert.contains(sel, 'env="test"')
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_loki_client_strips_trailing_slash() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ready"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ready"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local loki = require("assay.loki")
        local c = loki.client("{}///")
        local ok = c:ready()
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
