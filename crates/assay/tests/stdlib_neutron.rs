mod common;

use common::run_lua;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_neutron_agents_list_sends_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/agents"))
        .and(header("authorization", "Bearer nck_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "brand": "Demo",
            "agents": [{"id": "a1", "slug": "tom", "display_name": "Tom"}],
            "default_agent": {"stored": {}},
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        local out = c.agents:list()
        assert.eq(out.brand, "Demo")
        assert.eq(#out.agents, 1)
        assert.eq(out.agents[1].slug, "tom")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_neutron_secrets_set_and_list() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/admin/secrets/api-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "secrets": [{"name": "api-key", "has_value": true, "preview": "v1a2…l3"}],
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        local secrets = c.secrets:set("api-key", {{ value = "v1a2l3", agents = {{"tom"}} }})
        assert.eq(#secrets, 1)
        assert.eq(secrets[1].has_value, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_neutron_cf_access_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/tokens"))
        .and(header("cf-access-client-id", "cfid"))
        .and(header("cf-access-client-secret", "cfsecret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"tokens": []})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{
            token = "nck_test", cf_client_id = "cfid", cf_client_secret = "cfsecret",
        }})
        assert.eq(#c.tokens:list(), 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_neutron_http_error_surfaces() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/admin/connections/nope"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(serde_json::json!({"error": "not found"})),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        local ok, err = pcall(function() return c.connections:delete("nope") end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "HTTP 404")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_neutron_client_requires_token() {
    let script = r#"
        local neutron = require("assay.neutron")
        local ok, err = pcall(function() return neutron.client("http://x.example") end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "no token")
    "#;
    run_lua(script).await.unwrap();
}
