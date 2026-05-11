mod common;

use common::run_lua;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn api_key_header() -> &'static str {
    "API-Key test-name$test-secret"
}

fn api_key_lua_literal() -> &'static str {
    "test-name$test-secret"
}

#[tokio::test]
async fn test_health_ok() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local rauthy = require("assay.rauthy")
        local c = rauthy.client("{}", "{}")
        assert.eq(c.sys:health(), true)
        "#,
        server.uri(),
        api_key_lua_literal()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_health_false_on_5xx() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local rauthy = require("assay.rauthy")
        local c = rauthy.client("{}", "{}")
        assert.eq(c.sys:health(), false)
        "#,
        server.uri(),
        api_key_lua_literal()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_discovery_config() {
    let server = MockServer::start().await;
    let issuer = server.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/oidc/authorize"),
            "token_endpoint": format!("{issuer}/oidc/token"),
            "jwks_uri": format!("{issuer}/oidc/certs"),
            "id_token_signing_alg_values_supported": ["EdDSA", "RS256"]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local rauthy = require("assay.rauthy")
        local c = rauthy.client("{}", "{}")
        local cfg = c.discovery:config()
        assert.eq(cfg.issuer, "{}")
        assert.eq(cfg.token_endpoint, "{}/oidc/token")
        "#,
        server.uri(),
        api_key_lua_literal(),
        server.uri(),
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_clients_get_existing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/clients/openbao"))
        .and(header("Authorization", api_key_header()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "openbao",
            "name": "OpenBao",
            "confidential": true,
            "challenges": ["S256"]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local rauthy = require("assay.rauthy")
        local c = rauthy.client("{}", "{}")
        local got = c.clients:get("openbao")
        assert.eq(got.id, "openbao")
        assert.eq(got.confidential, true)
        assert.eq(got.challenges[1], "S256")
        "#,
        server.uri(),
        api_key_lua_literal()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_clients_get_missing_returns_nil() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/clients/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local rauthy = require("assay.rauthy")
        local c = rauthy.client("{}", "{}")
        assert.eq(c.clients:get("missing"), nil)
        "#,
        server.uri(),
        api_key_lua_literal()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_rotate_secret() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/clients/openbao/secret"))
        .and(header("Authorization", api_key_header()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "secret": "freshly-rotated-secret-value"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local rauthy = require("assay.rauthy")
        local c = rauthy.client("{}", "{}")
        local s = c.clients:rotate_secret("openbao")
        assert.eq(s, "freshly-rotated-secret-value")
        "#,
        server.uri(),
        api_key_lua_literal()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_reconcile_noop_when_no_drift() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/clients/openbao"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "openbao",
            "name": "OpenBao",
            "confidential": true,
            "challenges": ["S256"],
            "redirect_uris": ["https://o.example.com/cb"]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local rauthy = require("assay.rauthy")
        local c = rauthy.client("{}", "{}")
        local r = c.clients:reconcile({{
          id = "openbao", name = "OpenBao",
          confidential = true, challenges = {{ "S256" }},
          redirect_uris = {{ "https://o.example.com/cb" }},
        }})
        assert.eq(r.action, "noop")
        assert.eq(r.secret, nil)
        "#,
        server.uri(),
        api_key_lua_literal()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_reconcile_create_on_404() {
    let server = MockServer::start().await;
    // GET → 404
    Mock::given(method("GET"))
        .and(path("/clients/openbao"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    // POST /clients
    Mock::given(method("POST"))
        .and(path("/clients"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    // PUT /clients/openbao
    Mock::given(method("PUT"))
        .and(path("/clients/openbao"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    // POST /clients/openbao/secret → returns the minted secret
    Mock::given(method("POST"))
        .and(path("/clients/openbao/secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "secret": "minted-on-create"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local rauthy = require("assay.rauthy")
        local c = rauthy.client("{}", "{}")
        local r = c.clients:reconcile({{
          id = "openbao", name = "OpenBao",
          confidential = true, challenges = {{ "S256" }},
          redirect_uris = {{ "https://o.example.com/cb" }},
        }})
        assert.eq(r.action, "create")
        assert.eq(r.secret, "minted-on-create")
        "#,
        server.uri(),
        api_key_lua_literal()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_reconcile_rebuild_on_challenges_drift() {
    let server = MockServer::start().await;
    // Current state: challenges missing.
    Mock::given(method("GET"))
        .and(path("/clients/openbao"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "openbao",
            "name": "OpenBao",
            "confidential": true,
            "challenges": [],
            "redirect_uris": ["https://o.example.com/cb"]
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/clients/openbao"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/clients"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/clients/openbao"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/clients/openbao/secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "secret": "rotated-after-rebuild"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local rauthy = require("assay.rauthy")
        local c = rauthy.client("{}", "{}")
        local r = c.clients:reconcile({{
          id = "openbao", name = "OpenBao",
          confidential = true, challenges = {{ "S256" }},
          redirect_uris = {{ "https://o.example.com/cb" }},
        }})
        assert.eq(r.action, "rebuild")
        assert.eq(r.reason, "challenges-drift")
        assert.eq(r.secret, "rotated-after-rebuild")
        "#,
        server.uri(),
        api_key_lua_literal()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_reconcile_put_only_on_non_challenges_drift() {
    let server = MockServer::start().await;
    // Current state: id_token_alg differs but challenges OK.
    Mock::given(method("GET"))
        .and(path("/clients/openbao"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "openbao",
            "name": "OpenBao",
            "confidential": true,
            "challenges": ["S256"],
            "redirect_uris": ["https://o.example.com/cb"],
            "id_token_alg": "EdDSA"
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/clients/openbao"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local rauthy = require("assay.rauthy")
        local c = rauthy.client("{}", "{}")
        local r = c.clients:reconcile({{
          id = "openbao", name = "OpenBao",
          confidential = true, challenges = {{ "S256" }},
          redirect_uris = {{ "https://o.example.com/cb" }},
          id_token_alg = "RS256",
        }})
        assert.eq(r.action, "put")
        assert.eq(r.drift_on, "id_token_alg")
        assert.eq(r.secret, nil)
        "#,
        server.uri(),
        api_key_lua_literal()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_client_preset_openbao() {
    let script = r#"
        local rauthy = require("assay.rauthy")
        local p = rauthy.client_presets.openbao({ host = "openbao.example.com" })
        assert.eq(p.id, "openbao")
        assert.eq(p.confidential, true)
        assert.eq(p.id_token_alg, "RS256")
        assert.eq(p.access_token_alg, "RS256")
        assert.eq(p.challenges[1], "S256")
        assert.eq(p.redirect_uris[1], "https://openbao.example.com/ui/vault/auth/oidc/oidc/callback")
        assert.eq(p.redirect_uris[2], "http://localhost:8250/oidc/callback")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_client_preset_argocd() {
    let script = r#"
        local rauthy = require("assay.rauthy")
        local p = rauthy.client_presets.argocd({ host = "argocd.example.com" })
        assert.eq(p.id, "argocd")
        assert.eq(p.confidential, false)
        assert.eq(p.id_token_alg, "EdDSA")
        assert.eq(p.challenges[1], "S256")
        assert.eq(p.redirect_uris[1], "https://argocd.example.com/auth/callback")
        assert.eq(p.redirect_uris[2], "http://localhost:8085/auth/callback")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_client_preset_outline() {
    let script = r#"
        local rauthy = require("assay.rauthy")
        local p = rauthy.client_presets.outline({ host = "wiki.example.com" })
        assert.eq(p.id, "outline")
        assert.eq(p.name, "Outline")
        assert.eq(p.confidential, true)
        assert.eq(p.id_token_alg, "RS256")
        assert.eq(p.access_token_alg, "RS256")
        assert.eq(p.challenges[1], "S256")
        assert.eq(p.redirect_uris[1], "https://wiki.example.com/auth/oidc.callback")
        assert.eq(p.scopes[1], "openid")
        assert.eq(p.scopes[2], "email")
        assert.eq(p.scopes[3], "profile")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_client_preset_outline_requires_host() {
    let script = r#"
        local rauthy = require("assay.rauthy")
        local ok, err = pcall(rauthy.client_presets.outline, {})
        assert.eq(ok, false)
        assert.eq(string.find(err, "opts.host required") ~= nil, true)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_client_preset_openbao_requires_host() {
    let script = r#"
        local rauthy = require("assay.rauthy")
        local ok, err = pcall(rauthy.client_presets.openbao, {})
        assert.eq(ok, false)
        assert.eq(string.find(err, "opts.host required") ~= nil, true)
    "#;
    run_lua(script).await.unwrap();
}
