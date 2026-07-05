mod common;

use common::run_lua;
use wiremock::matchers::{header, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_sonarqube() {
    let script = r#"
        local sonarqube = require("assay.sonarqube")
        assert.not_nil(sonarqube)
        assert.not_nil(sonarqube.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_sonarqube_project_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/qualitygates/project_status"))
        .and(query_param("projectKey", "demo-project"))
        .and(header("Authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "projectStatus": {
                "status": "OK",
                "conditions": [
                    {"status": "OK", "metricKey": "new_coverage", "actualValue": "85.0"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local sonarqube = require("assay.sonarqube")
        local c = sonarqube.client("{}", {{ token = "test-token" }})
        local s = c.qualitygate:project_status("demo-project")
        assert.eq(s.projectStatus.status, "OK")
        assert.eq(#s.projectStatus.conditions, 1)
        assert.eq(s.projectStatus.conditions[1].metricKey, "new_coverage")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_sonarqube_project_status_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/qualitygates/project_status"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local sonarqube = require("assay.sonarqube")
        local c = sonarqube.client("{}", {{ token = "test-token" }})
        local s = c.qualitygate:project_status("missing")
        assert.eq(s, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_sonarqube_issues_search() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/issues/search"))
        .and(query_param("componentKeys", "demo-project"))
        .and(query_param("severities", "BLOCKER"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total": 2,
            "issues": [
                {"key": "AY-1", "severity": "BLOCKER", "type": "BUG"},
                {"key": "AY-2", "severity": "BLOCKER", "type": "VULNERABILITY"}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local sonarqube = require("assay.sonarqube")
        local c = sonarqube.client("{}", {{ token = "test-token" }})
        local r = c.issues:search({{ component_keys = "demo-project", severities = "BLOCKER" }})
        assert.eq(r.total, 2)
        assert.eq(#r.issues, 2)
        assert.eq(r.issues[1].key, "AY-1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_sonarqube_hotspots_search() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/hotspots/search"))
        .and(query_param("projectKey", "demo-project"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "paging": {"pageIndex": 1, "pageSize": 100, "total": 1},
            "hotspots": [
                {"key": "HS-1", "status": "TO_REVIEW", "vulnerabilityProbability": "HIGH"}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local sonarqube = require("assay.sonarqube")
        local c = sonarqube.client("{}", {{ token = "test-token" }})
        local r = c.hotspots:search({{ project_key = "demo-project" }})
        assert.eq(#r.hotspots, 1)
        assert.eq(r.hotspots[1].status, "TO_REVIEW")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_sonarqube_measures_component() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/measures/component"))
        .and(query_param("component", "demo-project"))
        .and(query_param("metricKeys", "coverage,bugs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "component": {
                "key": "demo-project",
                "measures": [
                    {"metric": "coverage", "value": "82.5"},
                    {"metric": "bugs", "value": "3"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local sonarqube = require("assay.sonarqube")
        local c = sonarqube.client("{}", {{ token = "test-token" }})
        local r = c.measures:component("demo-project", {{ "coverage", "bugs" }})
        assert.eq(r.component.key, "demo-project")
        assert.eq(#r.component.measures, 2)
        assert.eq(r.component.measures[1].metric, "coverage")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_sonarqube_projects_search() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/projects/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "paging": {"pageIndex": 1, "pageSize": 100, "total": 2},
            "components": [
                {"key": "demo-project", "name": "Demo Project"},
                {"key": "project-1", "name": "Project One"}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local sonarqube = require("assay.sonarqube")
        local c = sonarqube.client("{}", {{ token = "test-token" }})
        local r = c.projects:search({{ query = "demo" }})
        assert.eq(#r.components, 2)
        assert.eq(r.components[1].key, "demo-project")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_sonarqube_error_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/issues/search"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local sonarqube = require("assay.sonarqube")
        local c = sonarqube.client("{}", {{ token = "test-token" }})
        local ok, err = pcall(function() c.issues:search({{}}) end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "HTTP 500")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_sonarqube_basic_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/projects/search"))
        .and(header_exists("Authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "components": []
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local sonarqube = require("assay.sonarqube")
        local c = sonarqube.client("{}", {{ user = "squid", password = "s3cr3t" }})
        local r = c.projects:search()
        assert.eq(#r.components, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
