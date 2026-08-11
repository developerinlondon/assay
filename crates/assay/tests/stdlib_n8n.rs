mod common;

use common::run_lua;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_n8n() {
    let script = r#"
        local n8n = require("assay.n8n")
        assert.not_nil(n8n.client)
        assert.not_nil(n8n.all)
        assert.not_nil(n8n.wait)
        assert.not_nil(n8n.find_workflow_by_name)
        assert.not_nil(n8n.set_active)
        assert.not_nil(n8n.ensure_workflow)
        assert.not_nil(n8n.ensure_tag)
        assert.not_nil(n8n.ensure_workflow_tags)
        assert.not_nil(n8n.ensure_variable)
        assert.not_nil(n8n.ensure_project)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_n8n_client_sections() {
    let script = r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("http://example.invalid", { api_key = "k" })
        for _, section in ipairs({
            "workflows", "test_runs", "executions", "credentials", "tags", "variables",
            "projects", "folders", "users", "source_control", "audit", "data_tables",
            "community_packages", "settings", "log_streaming", "packages", "insights",
        }) do
            assert.not_nil(c[section], "missing section: " .. section)
        end
        assert.eq(type(c.discover), "function")
    "#;
    run_lua(script).await.unwrap();
}

// The API key travels in X-N8N-API-KEY, and every path is rooted at /api/v1.
// A mock that matches on both fails the request if either is wrong.
#[tokio::test]
async fn test_n8n_auth_header_and_base_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workflows"))
        .and(header("X-N8N-API-KEY", "secret-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "w1", "name": "One", "active": false}],
            "nextCursor": null
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "secret-key" }})
        local wfs = c.workflows:list()
        assert.eq(#wfs, 1)
        assert.eq(wfs[1].name, "One")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_n8n_workflows_page_exposes_cursor() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workflows"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "w1", "name": "One"}],
            "nextCursor": "cur-2"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local page = c.workflows:page({{ limit = 1 }})
        assert.eq(page.nextCursor, "cur-2")
        assert.eq(#page.data, 1)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_n8n_workflows_list_filters() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workflows"))
        .and(query_param("active", "true"))
        .and(query_param("tags", "prod"))
        .and(query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "w9", "name": "Nine", "active": true}],
            "nextCursor": null
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local wfs = c.workflows:list({{ active = true, tags = "prod", limit = 50 }})
        assert.eq(#wfs, 1)
        assert.eq(wfs[1].id, "w9")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_n8n_workflow_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workflows/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        assert.eq(c.workflows:get("missing"), nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_n8n_error_body_surfaces() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workflows"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "message": "Forbidden"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local ok, err = pcall(function() c.workflows:create({{ name = "x" }}) end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "403")
        assert.contains(tostring(err), "Forbidden")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

// An empty Lua table serialises as `{}`. n8n rejects a workflow whose `nodes`
// arrived as an object, so create/update must pin the empty list to `[]` while
// leaving connections/settings as objects. body_json fails the request unless
// the shape is exactly right.
#[tokio::test]
async fn test_n8n_create_workflow_pins_empty_node_list() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workflows"))
        .and(body_json(serde_json::json!({
            "name": "Blank",
            "nodes": [],
            "connections": {},
            "settings": {}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "w1", "name": "Blank", "active": false
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local wf = c.workflows:create({{ name = "Blank", nodes = {{}}, connections = {{}} }})
        assert.eq(wf.id, "w1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_n8n_create_workflow_keeps_supplied_connections() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workflows"))
        .and(body_json(serde_json::json!({
            "name": "Wired",
            "nodes": [{"id": "n1", "name": "Start"}],
            "connections": {"A": {"main": []}},
            "settings": {"executionOrder": "v1"}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "w2", "name": "Wired"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local wf = c.workflows:create({{
            name = "Wired",
            nodes = {{ {{ id = "n1", name = "Start" }} }},
            connections = {{ A = {{ main = json.array({{}}) }} }},
            settings = {{ executionOrder = "v1" }},
        }})
        assert.eq(wf.id, "w2")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_n8n_workflow_lifecycle_paths() {
    let server = MockServer::start().await;
    for (verb, suffix) in [
        ("POST", "activate"),
        ("POST", "deactivate"),
        ("POST", "archive"),
        ("POST", "unarchive"),
        ("POST", "publish"),
        ("POST", "unpublish"),
    ] {
        Mock::given(method(verb))
            .and(path(format!("/api/v1/workflows/w1/{suffix}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "w1", "op": suffix
            })))
            .mount(&server)
            .await;
    }
    Mock::given(method("PUT"))
        .and(path("/api/v1/workflows/w1/transfer"))
        .and(body_json(
            serde_json::json!({"destinationProjectId": "proj-7"}),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"op": "transfer"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/workflows/w1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "w1"})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        assert.eq(c.workflows:activate("w1").op, "activate")
        assert.eq(c.workflows:deactivate("w1").op, "deactivate")
        assert.eq(c.workflows:archive("w1").op, "archive")
        assert.eq(c.workflows:unarchive("w1").op, "unarchive")
        assert.eq(c.workflows:publish("w1").op, "publish")
        assert.eq(c.workflows:unpublish("w1").op, "unpublish")
        assert.eq(c.workflows:transfer("w1", "proj-7").op, "transfer")
        assert.eq(c.workflows:delete("w1").id, "w1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

// n8n takes tag assignments as an array of {id} objects, not bare strings.
#[tokio::test]
async fn test_n8n_set_workflow_tags_body_shape() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/workflows/w1/tags"))
        .and(body_json(serde_json::json!([{"id": "t1"}, {"id": "t2"}])))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"id": "t1", "name": "prod"},
            {"id": "t2", "name": "billing"}
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local tags = c.workflows:set_tags("w1", {{ "t1", "t2" }})
        assert.eq(#tags, 2)
        assert.eq(tags[1].name, "prod")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_n8n_executions_filters_and_actions() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/executions"))
        .and(query_param("status", "error"))
        .and(query_param("workflowId", "w1"))
        .and(query_param("limit", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": 42, "status": "error", "workflowId": "w1"}],
            "nextCursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/executions/42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 42})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/executions/42/stop"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"stopped": true})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/executions/42/retry"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"retried": true})),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local runs = c.executions:list({{ status = "error", workflowId = "w1", limit = 10 }})
        assert.eq(#runs, 1)
        assert.eq(runs[1].id, 42)
        assert.eq(c.executions:delete(42).id, 42)
        assert.eq(c.executions:stop(42).stopped, true)
        assert.eq(c.executions:retry(42).retried, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_n8n_credentials() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/credentials"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "c1", "name": "My HTTP", "type": "httpBasicAuth"
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/credentials/c1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "c1"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/credentials/schema/httpBasicAuth"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "type": "object",
            "properties": {"user": {"type": "string"}, "password": {"type": "string"}},
            "required": ["user", "password"]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local cred = c.credentials:create({{
            name = "My HTTP", type = "httpBasicAuth",
            data = {{ user = "u", password = "p" }},
        }})
        assert.eq(cred.id, "c1")
        local schema = c.credentials:schema("httpBasicAuth")
        assert.eq(schema.type, "object")
        assert.eq(#schema.required, 2)
        assert.eq(c.credentials:delete("c1").id, "c1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_n8n_source_control_and_audit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/source-control/pull"))
        .and(body_json(serde_json::json!({"force": true})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "variables": {"added": []}, "workflows": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/audit"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "Credentials Risk Report": {"risk": "credentials", "sections": []}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local pulled = c.source_control:pull({{ force = true }})
        assert.not_nil(pulled.variables)
        local report = c.audit:generate()
        assert.not_nil(report["Credentials Risk Report"])
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_n8n_projects_folders_users() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "p1", "name": "Ops"}], "nextCursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/p1/folders"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "f1", "name": "Nightly"}], "count": 1
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/api/v1/users/u1/role"))
        .and(body_json(
            serde_json::json!({"newRoleName": "global:admin"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local projects = c.projects:list()
        assert.eq(projects[1].name, "Ops")
        local folders = c.folders:list("p1")
        assert.eq(#folders, 1)
        assert.eq(folders[1].name, "Nightly")
        assert.eq(c.users:set_role("u1", "global:admin").ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
