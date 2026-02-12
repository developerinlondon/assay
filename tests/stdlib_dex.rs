mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn oidc_discovery_json(issuer: &str) -> serde_json::Value {
    serde_json::json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/auth"),
        "token_endpoint": format!("{issuer}/token"),
        "jwks_uri": format!("{issuer}/keys"),
        "userinfo_endpoint": format!("{issuer}/userinfo"),
        "revocation_endpoint": format!("{issuer}/revoke"),
        "scopes_supported": ["openid", "profile", "email", "groups", "offline_access"],
        "response_types_supported": ["code", "id_token", "token"],
        "grant_types_supported": ["authorization_code", "refresh_token", "client_credentials"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"]
    })
}

async fn mount_discovery(server: &MockServer) {
    let issuer = server.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(oidc_discovery_json(&issuer)))
        .mount(server)
        .await;
}

#[tokio::test]
async fn test_dex_discovery() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        local config = dex.discovery("{}")
        assert.eq(config.issuer, "{}")
        assert.eq(config.authorization_endpoint, "{}/auth")
        assert.eq(config.token_endpoint, "{}/token")
        assert.eq(config.jwks_uri, "{}/keys")
        assert.eq(config.userinfo_endpoint, "{}/userinfo")
        "#,
        server.uri(),
        server.uri(),
        server.uri(),
        server.uri(),
        server.uri(),
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_jwks() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;

    Mock::given(method("GET"))
        .and(path("/keys"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "keys": [{
                "kty": "RSA",
                "alg": "RS256",
                "use": "sig",
                "kid": "test-key-id",
                "n": "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM",
                "e": "AQAB"
            }]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        local jwks = dex.jwks("{}")
        assert.eq(#jwks.keys, 1)
        assert.eq(jwks.keys[1].kty, "RSA")
        assert.eq(jwks.keys[1].kid, "test-key-id")
        assert.eq(jwks.keys[1].alg, "RS256")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_issuer() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        local iss = dex.issuer("{}")
        assert.eq(iss, "{}")
        "#,
        server.uri(),
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_health_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/healthz"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        local ok = dex.health("{}")
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_health_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/healthz"))
        .respond_with(ResponseTemplate::new(503).set_body_string("unhealthy"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        local ok = dex.health("{}")
        assert.eq(ok, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_ready() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/healthz"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        local ok = dex.ready("{}")
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_has_endpoint_true() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        assert.eq(dex.has_endpoint("{}", "authorization_endpoint"), true)
        assert.eq(dex.has_endpoint("{}", "token_endpoint"), true)
        assert.eq(dex.has_endpoint("{}", "userinfo_endpoint"), true)
        "#,
        server.uri(),
        server.uri(),
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_has_endpoint_false() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        assert.eq(dex.has_endpoint("{}", "device_authorization_endpoint"), false)
        assert.eq(dex.has_endpoint("{}", "nonexistent_endpoint"), false)
        "#,
        server.uri(),
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_supported_scopes() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        local scopes = dex.supported_scopes("{}")
        assert.eq(#scopes, 5)
        assert.eq(scopes[1], "openid")
        assert.eq(scopes[2], "profile")
        assert.eq(scopes[3], "email")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_supports_scope_true() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        assert.eq(dex.supports_scope("{}", "openid"), true)
        assert.eq(dex.supports_scope("{}", "email"), true)
        assert.eq(dex.supports_scope("{}", "offline_access"), true)
        "#,
        server.uri(),
        server.uri(),
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_supports_scope_false() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        assert.eq(dex.supports_scope("{}", "phone"), false)
        assert.eq(dex.supports_scope("{}", "address"), false)
        "#,
        server.uri(),
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_supported_response_types() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        local types = dex.supported_response_types("{}")
        assert.eq(#types, 3)
        assert.eq(types[1], "code")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_supported_grant_types() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        local types = dex.supported_grant_types("{}")
        assert.eq(#types, 3)
        assert.eq(types[1], "authorization_code")
        assert.eq(types[2], "refresh_token")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_supports_grant_type_true() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        assert.eq(dex.supports_grant_type("{}", "authorization_code"), true)
        assert.eq(dex.supports_grant_type("{}", "refresh_token"), true)
        "#,
        server.uri(),
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_supports_grant_type_false() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        assert.eq(dex.supports_grant_type("{}", "implicit"), false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_validate_config_valid() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        local result = dex.validate_config("{}")
        assert.eq(result.ok, true)
        assert.eq(#result.errors, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_validate_config_invalid_missing_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": server.uri(),
            "scopes_supported": ["openid"]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        local result = dex.validate_config("{}")
        assert.eq(result.ok, false)
        assert.eq(#result.errors, 3)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_validate_config_issuer_mismatch() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": "https://wrong-issuer.example.com",
            "authorization_endpoint": format!("{}/auth", server.uri()),
            "token_endpoint": format!("{}/token", server.uri()),
            "jwks_uri": format!("{}/keys", server.uri())
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        local result = dex.validate_config("{}")
        assert.eq(result.ok, false)
        assert.eq(#result.errors, 1)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_admin_version_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/version"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "server": "dex",
            "version": "2.39.0"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        local ver = dex.admin_version("{}")
        assert.eq(ver.server, "dex")
        assert.eq(ver.version, "2.39.0")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_dex_admin_version_unavailable() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/version"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local dex = require("assay.dex")
        local ver = dex.admin_version("{}")
        assert.eq(ver, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
