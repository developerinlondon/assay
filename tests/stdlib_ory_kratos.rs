mod common;

use common::run_lua;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_kratos_require() {
    let script = r#"
        local kratos = require("assay.ory.kratos")
        assert.not_nil(kratos)
        assert.not_nil(kratos.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_kratos_whoami_authenticated() {
    let public = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/sessions/whoami"))
        .and(header("cookie", "ory_session_abc=xyz"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "session-id",
            "active": true,
            "identity": {
                "id": "user-id",
                "traits": {
                    "email": "alice@siemens.com",
                    "name": { "first": "Alice", "last": "Smith" }
                }
            }
        })))
        .mount(&public)
        .await;

    let script = format!(
        r#"
        local kratos = require("assay.ory.kratos")
        local k = kratos.client({{ public_url = "{}" }})
        local session = k:whoami("ory_session_abc=xyz")
        assert.not_nil(session)
        assert.eq(session.identity.traits.email, "alice@siemens.com")
        "#,
        public.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kratos_whoami_unauthenticated() {
    let public = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/sessions/whoami"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": { "code": 401, "message": "unauthenticated" }
        })))
        .mount(&public)
        .await;

    let script = format!(
        r#"
        local kratos = require("assay.ory.kratos")
        local k = kratos.client({{ public_url = "{}" }})
        local session = k:whoami("bogus=cookie")
        assert.eq(session, nil)
        "#,
        public.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kratos_get_identity() {
    let admin = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/admin/identities/user-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "user-123",
            "schema_id": "default",
            "traits": {
                "email": "bob@siemens.com",
                "name": { "first": "Bob", "last": "Jones" }
            }
        })))
        .mount(&admin)
        .await;

    let script = format!(
        r#"
        local kratos = require("assay.ory.kratos")
        local k = kratos.client({{ admin_url = "{}" }})
        local identity = k:get_identity("user-123")
        assert.eq(identity.id, "user-123")
        assert.eq(identity.traits.email, "bob@siemens.com")
        "#,
        admin.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kratos_list_identities() {
    let admin = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/admin/identities"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "id": "u1", "traits": { "email": "a@siemens.com" } },
            { "id": "u2", "traits": { "email": "b@siemens.com" } }
        ])))
        .mount(&admin)
        .await;

    let script = format!(
        r#"
        local kratos = require("assay.ory.kratos")
        local k = kratos.client({{ admin_url = "{}" }})
        local identities = k:list_identities()
        assert.eq(#identities, 2)
        "#,
        admin.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kratos_create_identity() {
    let admin = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/admin/identities"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "new-user-id",
            "schema_id": "default",
            "traits": { "email": "new@siemens.com" }
        })))
        .mount(&admin)
        .await;

    let script = format!(
        r#"
        local kratos = require("assay.ory.kratos")
        local k = kratos.client({{ admin_url = "{}" }})
        local identity = k:create_identity({{
          schema_id = "default",
          traits = {{ email = "new@siemens.com" }},
        }})
        assert.eq(identity.id, "new-user-id")
        "#,
        admin.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kratos_get_login_flow() {
    let public = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/self-service/login/flows"))
        .and(query_param("id", "flow-abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "flow-abc",
            "type": "browser",
            "oauth2_login_challenge": "challenge-xyz",
            "ui": {
                "action": "https://kratos.example.com/self-service/login?flow=flow-abc",
                "nodes": []
            }
        })))
        .mount(&public)
        .await;

    let script = format!(
        r#"
        local kratos = require("assay.ory.kratos")
        local k = kratos.client({{ public_url = "{}" }})
        local flow = k:get_login_flow("flow-abc")
        assert.eq(flow.id, "flow-abc")
        assert.eq(flow.oauth2_login_challenge, "challenge-xyz")
        "#,
        public.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kratos_ory_wrapper() {
    let script = r#"
        local ory = require("assay.ory")
        assert.not_nil(ory.kratos)
        assert.not_nil(ory.hydra)
        assert.not_nil(ory.keto)
        assert.not_nil(ory.connect)
    "#;
    run_lua(script).await.unwrap();
}
