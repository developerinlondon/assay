mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_prometheus_query_scalar() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": {
                "resultType": "vector",
                "result": [{"metric": {}, "value": [1234567890, "42"]}]
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
async fn test_prometheus_query_multi_result() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": {
                "resultType": "vector",
                "result": [
                    {"metric": {"instance": "host1:9090"}, "value": [1234567890, "1"]},
                    {"metric": {"instance": "host2:9090"}, "value": [1234567890, "0"]}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local prom = require("assay.prometheus")
        local results = prom.query("{}", "up")
        assert.eq(#results, 2)
        assert.eq(results[1].metric.instance, "host1:9090")
        assert.eq(results[1].value, 1)
        assert.eq(results[2].metric.instance, "host2:9090")
        assert.eq(results[2].value, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_prometheus_query_range() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/query_range"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": {
                "resultType": "matrix",
                "result": [{
                    "metric": {"__name__": "up"},
                    "values": [[1234567890, "1"], [1234567900, "1"]]
                }]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local prom = require("assay.prometheus")
        local result = prom.query_range("{}", "up", "1234567890", "1234567900", "10s")
        assert.eq(#result, 1)
        assert.eq(result[1].metric.__name__, "up")
        assert.eq(#result[1].values, 2)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_prometheus_alerts_active() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/alerts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": {
                "alerts": [{
                    "labels": {"alertname": "HighMemory", "severity": "critical"},
                    "annotations": {"summary": "Memory usage is above 90%"},
                    "state": "firing",
                    "activeAt": "2026-02-10T00:00:00Z"
                }]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local prom = require("assay.prometheus")
        local alerts = prom.alerts("{}")
        assert.eq(#alerts, 1)
        assert.eq(alerts[1].labels.alertname, "HighMemory")
        assert.eq(alerts[1].state, "firing")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_prometheus_alerts_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/alerts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": {"alerts": []}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local prom = require("assay.prometheus")
        local alerts = prom.alerts("{}")
        assert.eq(#alerts, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_prometheus_targets() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/targets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": {
                "activeTargets": [{
                    "discoveredLabels": {"job": "prometheus"},
                    "labels": {"instance": "localhost:9090"},
                    "scrapePool": "prometheus",
                    "scrapeUrl": "http://localhost:9090/metrics",
                    "health": "up"
                }],
                "droppedTargets": [{
                    "discoveredLabels": {"job": "dropped-job"}
                }]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local prom = require("assay.prometheus")
        local targets = prom.targets("{}")
        assert.eq(#targets.activeTargets, 1)
        assert.eq(targets.activeTargets[1].health, "up")
        assert.eq(targets.activeTargets[1].labels.instance, "localhost:9090")
        assert.eq(#targets.droppedTargets, 1)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_prometheus_rules() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/rules"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": {
                "groups": [{
                    "name": "example",
                    "rules": [{
                        "name": "HighMemory",
                        "type": "alerting",
                        "query": "node_memory_Active_bytes > 1e9",
                        "state": "firing"
                    }]
                }]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local prom = require("assay.prometheus")
        local groups = prom.rules("{}")
        assert.eq(#groups, 1)
        assert.eq(groups[1].name, "example")
        assert.eq(groups[1].rules[1].name, "HighMemory")
        assert.eq(groups[1].rules[1].type, "alerting")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_prometheus_label_values() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/label/job/values"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": ["prometheus", "node-exporter", "grafana"]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local prom = require("assay.prometheus")
        local values = prom.label_values("{}", "job")
        assert.eq(#values, 3)
        assert.eq(values[1], "prometheus")
        assert.eq(values[2], "node-exporter")
        assert.eq(values[3], "grafana")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_prometheus_series() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/series"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": [
                {"__name__": "up", "job": "prometheus", "instance": "localhost:9090"},
                {"__name__": "up", "job": "node", "instance": "localhost:9100"}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local prom = require("assay.prometheus")
        local series = prom.series("{}", {{"up"}})
        assert.eq(#series, 2)
        assert.eq(series[1].__name__, "up")
        assert.eq(series[1].job, "prometheus")
        assert.eq(series[2].instance, "localhost:9100")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_prometheus_config_reload_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/-/reload"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local prom = require("assay.prometheus")
        local ok = prom.config_reload("{}")
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_prometheus_config_reload_failure() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/-/reload"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local prom = require("assay.prometheus")
        local ok = prom.config_reload("{}")
        assert.eq(ok, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_prometheus_targets_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/targets/metadata"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": [{
                "target": {"instance": "localhost:9090", "job": "prometheus"},
                "metric": "prometheus_build_info",
                "type": "gauge",
                "help": "A metric with a constant 1 value.",
                "unit": ""
            }]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local prom = require("assay.prometheus")
        local meta = prom.targets_metadata("{}", {{
            match_target = "{{job=\"prometheus\"}}",
            metric = "prometheus_build_info",
            limit = "1",
        }})
        assert.eq(#meta, 1)
        assert.eq(meta[1].metric, "prometheus_build_info")
        assert.eq(meta[1].type, "gauge")
        assert.eq(meta[1].target.job, "prometheus")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
