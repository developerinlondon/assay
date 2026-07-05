mod common;

use common::run_lua;
use wiremock::matchers::{header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_servicenow() {
    let script = r#"
        local servicenow = require("assay.servicenow")
        assert.not_nil(servicenow)
        assert.not_nil(servicenow.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_servicenow_table_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/now/table/incident"))
        .and(query_param("sysparm_limit", "2"))
        .and(header_exists("Authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": [
                {"sys_id": "abc1", "number": "INC0001", "short_description": "disk full"},
                {"sys_id": "abc2", "number": "INC0002", "short_description": "cpu high"}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local servicenow = require("assay.servicenow")
        local c = servicenow.client("{}", {{ user = "admin", password = "secret" }})
        local rows = c.table:list("incident", {{ limit = 2 }})
        assert.eq(#rows, 2)
        assert.eq(rows[1].number, "INC0001")
        assert.eq(rows[2].sys_id, "abc2")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_servicenow_table_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/now/table/incident/abc1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": {"sys_id": "abc1", "number": "INC0001", "state": "2"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local servicenow = require("assay.servicenow")
        local c = servicenow.client("{}", {{ user = "admin", password = "secret" }})
        local row = c.table:get("incident", "abc1")
        assert.eq(row.number, "INC0001")
        assert.eq(row.state, "2")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_servicenow_table_get_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/now/table/incident/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local servicenow = require("assay.servicenow")
        local c = servicenow.client("{}", {{ user = "admin", password = "secret" }})
        local row = c.table:get("incident", "missing")
        assert.eq(row, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_servicenow_table_create() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/now/table/incident"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "result": {"sys_id": "new1", "number": "INC0100", "short_description": "new ticket"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local servicenow = require("assay.servicenow")
        local c = servicenow.client("{}", {{ user = "admin", password = "secret" }})
        local row = c.table:create("incident", {{ short_description = "new ticket" }})
        assert.eq(row.sys_id, "new1")
        assert.eq(row.number, "INC0100")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_servicenow_table_update() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/api/now/table/incident/abc1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": {"sys_id": "abc1", "state": "6"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local servicenow = require("assay.servicenow")
        local c = servicenow.client("{}", {{ user = "admin", password = "secret" }})
        local row = c.table:update("incident", "abc1", {{ state = "6" }})
        assert.eq(row.state, "6")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_servicenow_cmdb_query() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/now/cmdb/instance/cmdb_ci_server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": [
                {"sys_id": "ci1", "name": "app-1"},
                {"sys_id": "ci2", "name": "app-2"}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local servicenow = require("assay.servicenow")
        local c = servicenow.client("{}", {{ user = "admin", password = "secret" }})
        local rows = c.cmdb:query("cmdb_ci_server", {{ sysparm_query = "nameLIKEapp" }})
        assert.eq(#rows, 2)
        assert.eq(rows[1].name, "app-1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_servicenow_error_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/now/table/incident"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local servicenow = require("assay.servicenow")
        local c = servicenow.client("{}", {{ user = "admin", password = "wrong" }})
        local ok, err = pcall(function() c.table:list("incident") end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "HTTP 401")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
