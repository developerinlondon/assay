mod common;

use common::run_lua;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_grafana_health() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "commit": "abc123",
            "database": "ok",
            "version": "10.4.1"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local grafana = require("assay.grafana")
        local c = grafana.client("{}")
        local h = c:health()
        assert.eq(h.commit, "abc123")
        assert.eq(h.database, "ok")
        assert.eq(h.version, "10.4.1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_grafana_datasources() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/datasources"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": 1,
                "uid": "prometheus-uid",
                "name": "Prometheus",
                "type": "prometheus",
                "url": "http://prometheus:9090"
            },
            {
                "id": 2,
                "uid": "loki-uid",
                "name": "Loki",
                "type": "loki",
                "url": "http://loki:3100"
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local grafana = require("assay.grafana")
        local c = grafana.client("{}")
        local ds = c:datasources()
        assert.eq(#ds, 2)
        assert.eq(ds[1].name, "Prometheus")
        assert.eq(ds[1].uid, "prometheus-uid")
        assert.eq(ds[2].name, "Loki")
        assert.eq(ds[2].type, "loki")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_grafana_datasource_by_uid() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/datasources/uid/prometheus-uid"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 1,
            "uid": "prometheus-uid",
            "name": "Prometheus",
            "type": "prometheus",
            "url": "http://prometheus:9090",
            "access": "proxy",
            "isDefault": true
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local grafana = require("assay.grafana")
        local c = grafana.client("{}")
        local ds = c:datasource("prometheus-uid")
        assert.eq(ds.id, 1)
        assert.eq(ds.uid, "prometheus-uid")
        assert.eq(ds.name, "Prometheus")
        assert.eq(ds.isDefault, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_grafana_search() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": 1,
                "uid": "dash-abc",
                "title": "Node Exporter",
                "type": "dash-db",
                "uri": "db/node-exporter",
                "url": "/d/dash-abc/node-exporter",
                "tags": ["linux", "monitoring"]
            },
            {
                "id": 2,
                "uid": "dash-def",
                "title": "Kubernetes Overview",
                "type": "dash-db",
                "uri": "db/kubernetes-overview",
                "url": "/d/dash-def/kubernetes-overview",
                "tags": ["kubernetes"]
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local grafana = require("assay.grafana")
        local c = grafana.client("{}")
        local results = c:search({{ query = "node" }})
        assert.eq(#results, 2)
        assert.eq(results[1].title, "Node Exporter")
        assert.eq(results[1].uid, "dash-abc")
        assert.eq(results[2].title, "Kubernetes Overview")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_grafana_dashboard() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/dashboards/uid/dash-abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "dashboard": {
                "id": 1,
                "uid": "dash-abc",
                "title": "Node Exporter",
                "panels": [
                    {"id": 1, "title": "CPU Usage", "type": "graph"},
                    {"id": 2, "title": "Memory Usage", "type": "graph"}
                ],
                "version": 3
            },
            "meta": {
                "isStarred": false,
                "slug": "node-exporter",
                "folderId": 10,
                "folderUid": "infra",
                "folderTitle": "Infrastructure"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local grafana = require("assay.grafana")
        local c = grafana.client("{}")
        local result = c:dashboard("dash-abc")
        assert.eq(result.dashboard.title, "Node Exporter")
        assert.eq(result.dashboard.uid, "dash-abc")
        assert.eq(#result.dashboard.panels, 2)
        assert.eq(result.meta.folderTitle, "Infrastructure")
        assert.eq(result.meta.slug, "node-exporter")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_grafana_annotations() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/annotations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": 1,
                "dashboardId": 100,
                "panelId": 5,
                "text": "Deployment v1.2.3",
                "tags": ["deploy"],
                "time": 1700000000000_i64,
                "timeEnd": 1700000060000_i64
            },
            {
                "id": 2,
                "dashboardId": 100,
                "panelId": 5,
                "text": "Deployment v1.2.4",
                "tags": ["deploy"],
                "time": 1700001000000_i64,
                "timeEnd": 1700001060000_i64
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local grafana = require("assay.grafana")
        local c = grafana.client("{}")
        local anns = c:annotations({{ from = 1700000000000, to = 1700002000000, limit = 10 }})
        assert.eq(#anns, 2)
        assert.eq(anns[1].text, "Deployment v1.2.3")
        assert.eq(anns[1].tags[1], "deploy")
        assert.eq(anns[2].text, "Deployment v1.2.4")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_grafana_create_annotation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/annotations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 42,
            "message": "Annotation added"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local grafana = require("assay.grafana")
        local c = grafana.client("{}")
        local result = c:create_annotation({{
            dashboardId = 100,
            panelId = 5,
            text = "Deployment started",
            tags = {{"deploy", "ci"}},
            time = 1700000000000,
            timeEnd = 1700000060000,
        }})
        assert.eq(result.id, 42)
        assert.eq(result.message, "Annotation added")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_grafana_org() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/org"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 1,
            "name": "Main Org.",
            "address": {
                "address1": "",
                "address2": "",
                "city": "",
                "zipCode": "",
                "state": "",
                "country": ""
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local grafana = require("assay.grafana")
        local c = grafana.client("{}")
        local org = c:org()
        assert.eq(org.id, 1)
        assert.eq(org.name, "Main Org.")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_grafana_alert_rules() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/provisioning/alert-rules"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": 1,
                "uid": "rule-abc",
                "orgID": 1,
                "folderUID": "alerts",
                "ruleGroup": "critical",
                "title": "High CPU Usage",
                "condition": "C",
                "noDataState": "NoData",
                "execErrState": "Error"
            },
            {
                "id": 2,
                "uid": "rule-def",
                "orgID": 1,
                "folderUID": "alerts",
                "ruleGroup": "warning",
                "title": "Disk Space Low",
                "condition": "C",
                "noDataState": "NoData",
                "execErrState": "Error"
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local grafana = require("assay.grafana")
        local c = grafana.client("{}")
        local rules = c:alert_rules()
        assert.eq(#rules, 2)
        assert.eq(rules[1].title, "High CPU Usage")
        assert.eq(rules[1].uid, "rule-abc")
        assert.eq(rules[2].title, "Disk Space Low")
        assert.eq(rules[2].ruleGroup, "warning")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_grafana_folders() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/folders"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": 1,
                "uid": "infra",
                "title": "Infrastructure",
                "url": "/dashboards/f/infra/infrastructure"
            },
            {
                "id": 2,
                "uid": "apps",
                "title": "Applications",
                "url": "/dashboards/f/apps/applications"
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local grafana = require("assay.grafana")
        local c = grafana.client("{}")
        local folders = c:folders()
        assert.eq(#folders, 2)
        assert.eq(folders[1].title, "Infrastructure")
        assert.eq(folders[1].uid, "infra")
        assert.eq(folders[2].title, "Applications")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_grafana_api_key_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/org"))
        .and(header("Authorization", "Bearer glsa_test_key_12345"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 1,
            "name": "Main Org."
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local grafana = require("assay.grafana")
        local c = grafana.client("{}", {{ api_key = "glsa_test_key_12345" }})
        local org = c:org()
        assert.eq(org.id, 1)
        assert.eq(org.name, "Main Org.")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
