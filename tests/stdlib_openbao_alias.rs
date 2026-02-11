mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_openbao_alias_loads() {
    let script = r#"
        local bao = require("assay.openbao")
        assert.not_nil(bao)
        assert.not_nil(bao.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_openbao_alias_matches_vault() {
    let script = r#"
        local vault = require("assay.vault")
        local bao = require("assay.openbao")
        if vault ~= bao then
            error("openbao module should be the same table as vault module")
        end
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_openbao_alias_client_works() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {"data": {"key": "value"}}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local bao = require("assay.openbao")
        local c = bao.client("{}", "test-token")
        local data = c:kv_get("secret", "test")
        assert.eq(data.data.key, "value")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
