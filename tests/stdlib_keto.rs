mod common;

use common::run_lua;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_keto_require() {
    let script = r#"
        local keto = require("assay.keto")
        assert.not_nil(keto)
        assert.not_nil(keto.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_keto_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/relation-tuples"))
        .and(query_param("namespace", "Role"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "relation_tuples": [
                {
                    "namespace": "Role",
                    "object": "namespace1:role-a",
                    "relation": "members",
                    "subject_id": "user:alice"
                }
            ],
            "next_page_token": ""
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local keto = require("assay.keto")
        local k = keto.client("{}")
        local result = k:list({{ namespace = "Role" }})
        assert.eq(#result.relation_tuples, 1)
        assert.eq(result.relation_tuples[1].object, "namespace1:role-a")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_keto_check_allowed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/relation-tuples/check"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "allowed": true })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local keto = require("assay.keto")
        local k = keto.client("{}")
        local ok = k:check("apps", "cc", "admin", "user:alice")
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_keto_check_denied() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/relation-tuples/check"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "allowed": false })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local keto = require("assay.keto")
        local k = keto.client("{}")
        local ok = k:check("apps", "cc", "admin", "user:bob")
        assert.eq(ok, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_keto_get_user_roles() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/relation-tuples"))
        .and(query_param("namespace", "Role"))
        .and(query_param("relation", "members"))
        .and(query_param("subject_id", "user:alice"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "relation_tuples": [
                {
                    "namespace": "Role",
                    "object": "namespace1:role-a",
                    "relation": "members",
                    "subject_id": "user:alice"
                },
                {
                    "namespace": "Role",
                    "object": "namespace2:role-a",
                    "relation": "members",
                    "subject_id": "user:alice"
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local keto = require("assay.keto")
        local k = keto.client("{}")
        local roles = k:get_user_roles("alice")
        assert.eq(#roles, 2)
        assert.eq(roles[1].object, "namespace1:role-a")
        assert.eq(roles[2].object, "namespace2:role-a")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_keto_user_has_any_role() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/relation-tuples"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "relation_tuples": [
                {
                    "namespace": "Role",
                    "object": "namespace2:role-b",
                    "relation": "members",
                    "subject_id": "user:bob"
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local keto = require("assay.keto")
        local k = keto.client("{}")
        local has_admin = k:user_has_any_role("bob", {{"namespace1:role-a", "namespace2:role-a"}})
        assert.eq(has_admin, false)
        local has_op = k:user_has_any_role("bob", {{"namespace2:role-b"}})
        assert.eq(has_op, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_keto_expand() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/relation-tuples/expand"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "type": "union",
            "tuple": {
                "namespace": "Role",
                "object": "namespace1:role-a",
                "relation": "members"
            },
            "children": []
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local keto = require("assay.keto")
        local k = keto.client("{}")
        local tree = k:expand("Role", "namespace1:role-a", "members")
        assert.eq(tree.type, "union")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_keto_create_tuple() {
    let read_server = MockServer::start().await;
    let write_server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/admin/relation-tuples"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({})))
        .mount(&write_server)
        .await;

    let script = format!(
        r#"
        local keto = require("assay.keto")
        local k = keto.client("{}", {{ write_url = "{}" }})
        k:create({{
          namespace = "Role",
          object = "namespace2:role-c",
          relation = "members",
          subject_id = "user:carol",
        }})
        "#,
        read_server.uri(),
        write_server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_keto_write_requires_write_url() {
    let read_server = MockServer::start().await;

    let script = format!(
        r#"
        local keto = require("assay.keto")
        local k = keto.client("{}")
        local ok, err = pcall(function()
          k:create({{ namespace = "Role", object = "x", relation = "members", subject_id = "user:y" }})
        end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "write_url not configured")
        "#,
        read_server.uri()
    );
    run_lua(&script).await.unwrap();
}
