mod common;

use common::run_lua;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

// M.all must follow nextCursor rather than stopping at the first page.
#[tokio::test]
async fn test_n8n_all_follows_cursor() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workflows"))
        .and(query_param("cursor", "page2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "w3", "name": "Three"}],
            "nextCursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workflows"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "w1", "name": "One"}, {"id": "w2", "name": "Two"}],
            "nextCursor": "page2"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local all = n8n.all(c.workflows)
        assert.eq(#all, 3)
        assert.eq(all[1].id, "w1")
        assert.eq(all[3].id, "w3")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

// The `name` list filter is a server-side substring match, so the helper has
// to reject near-misses itself.
#[tokio::test]
async fn test_n8n_find_workflow_by_name_is_exact() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workflows"))
        .and(query_param("name", "Nightly"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                {"id": "w1", "name": "Nightly Backup"},
                {"id": "w2", "name": "Nightly"}
            ],
            "nextCursor": null
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local wf = n8n.find_workflow_by_name(c, "Nightly")
        assert.eq(wf.id, "w2")
        assert.eq(wf.name, "Nightly")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_n8n_find_workflow_by_name_absent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workflows"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "w1", "name": "Nightly Backup"}],
            "nextCursor": null
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        assert.eq(n8n.find_workflow_by_name(c, "Nightly"), nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

// A workflow with no name match is created, never PUT to a guessed ID.
#[tokio::test]
async fn test_n8n_ensure_workflow_creates_when_absent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workflows"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [], "nextCursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workflows"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "new-1", "name": "Nightly", "active": false
        })))
        .expect(1)
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local wf = n8n.ensure_workflow(c, {{ name = "Nightly", nodes = {{}} }})
        assert.eq(wf.id, "new-1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
    server.verify().await;
}

// An existing name is replaced in place: PUT to the found ID, and no POST.
#[tokio::test]
async fn test_n8n_ensure_workflow_updates_existing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workflows"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "old-9", "name": "Nightly", "active": false}],
            "nextCursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/workflows/old-9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "old-9", "name": "Nightly", "active": false
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workflows"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "wrong"})))
        .expect(0)
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local wf = n8n.ensure_workflow(c, {{ name = "Nightly", nodes = {{}} }})
        assert.eq(wf.id, "old-9")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
    server.verify().await;
}

#[tokio::test]
async fn test_n8n_ensure_workflow_requires_name() {
    let script = r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("http://example.invalid", { api_key = "k" })
        local ok = pcall(function() n8n.ensure_workflow(c, { nodes = {} }) end)
        assert.eq(ok, false)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_n8n_ensure_workflow_activates() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workflows"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [], "nextCursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workflows"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "new-1", "name": "Nightly", "active": false
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workflows/new-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "new-1", "name": "Nightly", "active": false
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workflows/new-1/activate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "new-1", "name": "Nightly", "active": true
        })))
        .expect(1)
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local wf = n8n.ensure_workflow(c, {{ name = "Nightly", nodes = {{}} }}, {{ active = true }})
        assert.eq(wf.active, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
    server.verify().await;
}

// Reconciling to the state the workflow is already in must issue no write.
#[tokio::test]
async fn test_n8n_set_active_is_a_noop_when_already_active() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workflows/w1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "w1", "name": "Nightly", "active": true
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workflows/w1/activate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(0)
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local wf = n8n.set_active(c, "w1", true)
        assert.eq(wf.active, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
    server.verify().await;
}

#[tokio::test]
async fn test_n8n_set_active_deactivates() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workflows/w1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "w1", "active": true
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workflows/w1/deactivate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "w1", "active": false
        })))
        .expect(1)
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        assert.eq(n8n.set_active(c, "w1", false).active, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
    server.verify().await;
}

#[tokio::test]
async fn test_n8n_ensure_tag_reuses_existing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "t1", "name": "prod"}], "nextCursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "wrong"})))
        .expect(0)
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        assert.eq(n8n.ensure_tag(c, "prod").id, "t1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
    server.verify().await;
}

#[tokio::test]
async fn test_n8n_ensure_tag_creates_when_missing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "t1", "name": "staging"}], "nextCursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tags"))
        .and(body_json(serde_json::json!({"name": "prod"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "t2", "name": "prod"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        assert.eq(n8n.ensure_tag(c, "prod").id, "t2")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
    server.verify().await;
}

#[tokio::test]
async fn test_n8n_ensure_workflow_tags_creates_then_assigns() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "t1", "name": "prod"}], "nextCursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tags"))
        .and(body_json(serde_json::json!({"name": "billing"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "t2", "name": "billing"
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/workflows/w1/tags"))
        .and(body_json(serde_json::json!([{"id": "t1"}, {"id": "t2"}])))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"id": "t1", "name": "prod"}, {"id": "t2", "name": "billing"}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local tags = n8n.ensure_workflow_tags(c, "w1", {{ "prod", "billing" }})
        assert.eq(#tags, 2)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
    server.verify().await;
}

// Same key and same value: nothing is written.
#[tokio::test]
async fn test_n8n_ensure_variable_noop_when_current() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/variables"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "v1", "key": "API_HOST", "value": "https://a"}],
            "nextCursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/variables/v1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/variables"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(0)
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        assert.eq(n8n.ensure_variable(c, "API_HOST", "https://a").id, "v1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
    server.verify().await;
}

// Same key, different value: update in place, never create a duplicate.
#[tokio::test]
async fn test_n8n_ensure_variable_updates_changed_value() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/variables"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "v1", "key": "API_HOST", "value": "https://old"}],
            "nextCursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/variables/v1"))
        .and(body_json(
            serde_json::json!({"key": "API_HOST", "value": "https://new"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "v1", "key": "API_HOST", "value": "https://new"
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/variables"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(0)
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        local v = n8n.ensure_variable(c, "API_HOST", "https://new")
        assert.eq(v.value, "https://new")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
    server.verify().await;
}

#[tokio::test]
async fn test_n8n_ensure_variable_creates_when_missing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/variables"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [], "nextCursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/variables"))
        .and(body_json(
            serde_json::json!({"key": "API_HOST", "value": "https://a"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "v9", "key": "API_HOST", "value": "https://a"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        assert.eq(n8n.ensure_variable(c, "API_HOST", "https://a").id, "v9")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
    server.verify().await;
}

#[tokio::test]
async fn test_n8n_ensure_project_reuses_existing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "p1", "name": "Ops"}], "nextCursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(0)
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        assert.eq(n8n.ensure_project(c, "Ops").id, "p1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
    server.verify().await;
}

#[tokio::test]
async fn test_n8n_ensure_project_creates_when_missing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [], "nextCursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/projects"))
        .and(body_json(
            serde_json::json!({"name": "Ops", "type": "team"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "p9", "name": "Ops"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        local c = n8n.client("{}", {{ api_key = "k" }})
        assert.eq(n8n.ensure_project(c, "Ops", {{ type = "team" }}).id, "p9")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
    server.verify().await;
}

#[tokio::test]
async fn test_n8n_wait_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/healthz"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "ok"})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        assert.eq(n8n.wait("{}", {{ timeout = 5, interval = 0.1 }}), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_n8n_wait_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/healthz"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local n8n = require("assay.n8n")
        n8n.wait("{}", {{ timeout = 1, interval = 0.5 }})
        "#,
        server.uri()
    );
    assert!(run_lua(&script).await.is_err());
}
