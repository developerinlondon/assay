mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_ecr() {
    let script = r#"
        local ecr = require("assay.aws.ecr")
        assert.not_nil(ecr)
        assert.not_nil(ecr.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_ecr_get_authorization_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"authorizationData":[{"authorizationToken":"QVdTOnBhc3N3b3Jk","proxyEndpoint":"https://123456789012.dkr.ecr.us-east-1.amazonaws.com","expiresAt":"2026-01-01T00:00:00Z"}]}"#,
        ))
        .mount(&server)
        .await;

    let host = server.uri().replace("http://", "");

    let script = format!(
        r#"
        local ecr = require("assay.aws.ecr")
        local c = ecr.client("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "", "us-east-1")
        -- Override host for testing
        local result = c:get_authorization_token()
        assert.eq(result.token, "password")
        assert.not_nil(result.proxy_endpoint)
        assert.not_nil(result.expires_at)
        "#
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_ecr_get_authorization_token_errors_on_non_200() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(403).set_body_string(r#"{"__type":"AccessDeniedException"}"#))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local ecr = require("assay.aws.ecr")
        local c = ecr.client("BADKEY", "BADSECRET", "", "us-east-1")
        local ok = pcall(function() c:get_authorization_token() end)
        assert.eq(ok, false)
        "#
    );
    run_lua(&script).await.unwrap();
}
