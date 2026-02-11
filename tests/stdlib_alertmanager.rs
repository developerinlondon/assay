mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_alertmanager_alerts_active() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/alerts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "annotations": {"summary": "Memory usage is above 90%"},
                "endsAt": "2026-02-10T01:00:00.000Z",
                "fingerprint": "abc123",
                "receivers": [{"name": "default"}],
                "startsAt": "2026-02-10T00:00:00.000Z",
                "status": {"inhibitedBy": [], "silencedBy": [], "state": "active"},
                "updatedAt": "2026-02-10T00:00:00.000Z",
                "labels": {"alertname": "HighMemory", "severity": "critical"}
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local am = require("assay.alertmanager")
        local alerts = am.alerts("{}")
        assert.eq(#alerts, 1)
        assert.eq(alerts[1].labels.alertname, "HighMemory")
        assert.eq(alerts[1].labels.severity, "critical")
        assert.eq(alerts[1].status.state, "active")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_alertmanager_alerts_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/alerts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local am = require("assay.alertmanager")
        local alerts = am.alerts("{}")
        assert.eq(#alerts, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_alertmanager_post_alerts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/alerts"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local am = require("assay.alertmanager")
        local ok = am.post_alerts("{}", {{{{
            labels = {{ alertname = "TestAlert", severity = "warning" }},
            annotations = {{ summary = "Test alert fired" }},
            startsAt = "2026-02-10T00:00:00.000Z",
            endsAt = "2026-02-10T01:00:00.000Z",
            generatorURL = "http://localhost:9090/graph",
        }}}})
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_alertmanager_alert_groups() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/alerts/groups"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "labels": {"alertname": "HighMemory"},
                "receiver": {"name": "default"},
                "alerts": [
                    {
                        "annotations": {},
                        "endsAt": "2026-02-10T01:00:00.000Z",
                        "fingerprint": "abc123",
                        "receivers": [{"name": "default"}],
                        "startsAt": "2026-02-10T00:00:00.000Z",
                        "status": {"inhibitedBy": [], "silencedBy": [], "state": "active"},
                        "updatedAt": "2026-02-10T00:00:00.000Z",
                        "labels": {"alertname": "HighMemory", "instance": "host1"}
                    }
                ]
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local am = require("assay.alertmanager")
        local groups = am.alert_groups("{}")
        assert.eq(#groups, 1)
        assert.eq(groups[1].labels.alertname, "HighMemory")
        assert.eq(#groups[1].alerts, 1)
        assert.eq(groups[1].receiver.name, "default")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_alertmanager_silences_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/silences"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": "silence-001",
                "status": {"state": "active"},
                "updatedAt": "2026-02-10T00:00:00.000Z",
                "comment": "Maintenance window",
                "createdBy": "admin",
                "endsAt": "2026-02-10T02:00:00.000Z",
                "startsAt": "2026-02-10T00:00:00.000Z",
                "matchers": [
                    {"isEqual": true, "isRegex": false, "name": "alertname", "value": "HighMemory"}
                ]
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local am = require("assay.alertmanager")
        local silences = am.silences("{}")
        assert.eq(#silences, 1)
        assert.eq(silences[1].id, "silence-001")
        assert.eq(silences[1].createdBy, "admin")
        assert.eq(silences[1].matchers[1].name, "alertname")
        assert.eq(silences[1].matchers[1].value, "HighMemory")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_alertmanager_silence_by_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/silence/silence-001"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "silence-001",
            "status": {"state": "active"},
            "updatedAt": "2026-02-10T00:00:00.000Z",
            "comment": "Maintenance window",
            "createdBy": "admin",
            "endsAt": "2026-02-10T02:00:00.000Z",
            "startsAt": "2026-02-10T00:00:00.000Z",
            "matchers": [
                {"isEqual": true, "isRegex": false, "name": "alertname", "value": "HighMemory"}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local am = require("assay.alertmanager")
        local s = am.silence("{}", "silence-001")
        assert.eq(s.id, "silence-001")
        assert.eq(s.comment, "Maintenance window")
        assert.eq(s.status.state, "active")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_alertmanager_create_silence() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/silences"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"silenceID": "new-silence-123"})),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local am = require("assay.alertmanager")
        local result = am.create_silence("{}", {{
            matchers = {{{{ name = "alertname", value = "HighMemory", isRegex = false, isEqual = true }}}},
            startsAt = "2026-02-10T00:00:00.000Z",
            endsAt = "2026-02-10T02:00:00.000Z",
            createdBy = "admin",
            comment = "Maintenance window",
        }})
        assert.eq(result.silenceID, "new-silence-123")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_alertmanager_delete_silence() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v2/silence/silence-001"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local am = require("assay.alertmanager")
        local ok = am.delete_silence("{}", "silence-001")
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_alertmanager_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "cluster": {
                "name": "cluster-1",
                "status": "ready",
                "peers": [
                    {"name": "peer-1", "address": "10.0.0.1:9094"}
                ]
            },
            "config": {
                "original": "global:\n  resolve_timeout: 5m"
            },
            "uptime": "2026-02-09T00:00:00.000Z",
            "versionInfo": {
                "branch": "HEAD",
                "buildDate": "2026-01-01T00:00:00Z",
                "buildUser": "ci",
                "goVersion": "go1.22.0",
                "revision": "abc123",
                "version": "0.27.0"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local am = require("assay.alertmanager")
        local st = am.status("{}")
        assert.eq(st.cluster.status, "ready")
        assert.eq(st.versionInfo.version, "0.27.0")
        assert.eq(st.cluster.name, "cluster-1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_alertmanager_receivers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/receivers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"name": "default"},
            {"name": "slack-critical"},
            {"name": "pagerduty"}
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local am = require("assay.alertmanager")
        local recv = am.receivers("{}")
        assert.eq(#recv, 3)
        assert.eq(recv[1].name, "default")
        assert.eq(recv[2].name, "slack-critical")
        assert.eq(recv[3].name, "pagerduty")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_alertmanager_is_firing_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/alerts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "annotations": {},
                "endsAt": "2026-02-10T01:00:00.000Z",
                "fingerprint": "abc123",
                "receivers": [{"name": "default"}],
                "startsAt": "2026-02-10T00:00:00.000Z",
                "status": {"inhibitedBy": [], "silencedBy": [], "state": "active"},
                "updatedAt": "2026-02-10T00:00:00.000Z",
                "labels": {"alertname": "HighMemory", "severity": "critical"}
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local am = require("assay.alertmanager")
        local firing = am.is_firing("{}", "HighMemory")
        assert.eq(firing, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_alertmanager_is_firing_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/alerts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local am = require("assay.alertmanager")
        local firing = am.is_firing("{}", "NonExistent")
        assert.eq(firing, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_alertmanager_active_count() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/alerts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "annotations": {},
                "endsAt": "2026-02-10T01:00:00.000Z",
                "fingerprint": "abc123",
                "receivers": [{"name": "default"}],
                "startsAt": "2026-02-10T00:00:00.000Z",
                "status": {"inhibitedBy": [], "silencedBy": [], "state": "active"},
                "updatedAt": "2026-02-10T00:00:00.000Z",
                "labels": {"alertname": "HighMemory"}
            },
            {
                "annotations": {},
                "endsAt": "2026-02-10T01:00:00.000Z",
                "fingerprint": "def456",
                "receivers": [{"name": "default"}],
                "startsAt": "2026-02-10T00:00:00.000Z",
                "status": {"inhibitedBy": [], "silencedBy": [], "state": "active"},
                "updatedAt": "2026-02-10T00:00:00.000Z",
                "labels": {"alertname": "HighCPU"}
            },
            {
                "annotations": {},
                "endsAt": "2026-02-10T01:00:00.000Z",
                "fingerprint": "ghi789",
                "receivers": [{"name": "default"}],
                "startsAt": "2026-02-10T00:00:00.000Z",
                "status": {"inhibitedBy": [], "silencedBy": [], "state": "active"},
                "updatedAt": "2026-02-10T00:00:00.000Z",
                "labels": {"alertname": "DiskFull"}
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local am = require("assay.alertmanager")
        local count = am.active_count("{}")
        assert.eq(count, 3)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_alertmanager_silence_alert() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/silences"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"silenceID": "auto-silence-456"})),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local am = require("assay.alertmanager")
        local sid = am.silence_alert("{}", "HighMemory", 2, {{
            created_by = "test-user",
            comment = "Silencing for maintenance",
        }})
        assert.eq(sid, "auto-silence-456")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
