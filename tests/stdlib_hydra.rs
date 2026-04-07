mod common;

use common::run_lua;
use wiremock::matchers::{body_string_contains, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_hydra_require() {
    let script = r#"
        local hydra = require("assay.hydra")
        assert.not_nil(hydra)
        assert.not_nil(hydra.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_hydra_list_clients() {
    let admin = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/admin/clients"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "client_id": "cc", "client_name": "Command Center" },
            { "client_id": "temporal", "client_name": "Temporal" }
        ])))
        .mount(&admin)
        .await;

    let script = format!(
        r#"
        local hydra = require("assay.hydra")
        local h = hydra.client({{ admin_url = "{}" }})
        local clients = h:list_clients()
        assert.eq(#clients, 2)
        assert.eq(clients[1].client_id, "cc")
        "#,
        admin.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_hydra_update_client() {
    let admin = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/admin/clients/cc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "client_id": "cc",
            "client_name": "Command Center",
            "token_endpoint_auth_method": "client_secret_post"
        })))
        .mount(&admin)
        .await;

    let script = format!(
        r#"
        local hydra = require("assay.hydra")
        local h = hydra.client({{ admin_url = "{}" }})
        local client = h:update_client("cc", {{
          client_name = "Command Center",
          client_secret = "secret",
          grant_types = {{"authorization_code", "refresh_token"}},
          response_types = {{"code"}},
          scope = "openid profile email",
          redirect_uris = {{"https://cc.example.com/auth/callback"}},
          token_endpoint_auth_method = "client_secret_post",
        }})
        assert.eq(client.client_id, "cc")
        "#,
        admin.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_hydra_build_authorize_url() {
    let script = r#"
        local hydra = require("assay.hydra")
        local h = hydra.client({ public_url = "https://hydra.example.com" })
        local url = h:build_authorize_url("cc", {
          redirect_uri = "https://cc.example.com/auth/callback",
          scope = "openid profile email",
          state = "xyz",
        })
        assert.contains(url, "https://hydra.example.com/oauth2/auth?")
        assert.contains(url, "client_id=cc")
        assert.contains(url, "response_type=code")
        assert.contains(url, "state=xyz")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_hydra_exchange_code() {
    let public = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "access.jwt",
            "id_token": "id.jwt",
            "refresh_token": "refresh.opaque",
            "token_type": "bearer",
            "expires_in": 3600
        })))
        .mount(&public)
        .await;

    let script = format!(
        r#"
        local hydra = require("assay.hydra")
        local h = hydra.client({{ public_url = "{}" }})
        local tokens = h:exchange_code({{
          code = "abc",
          redirect_uri = "https://cc.example.com/auth/callback",
          client_id = "cc",
          client_secret = "secret",
        }})
        assert.eq(tokens.access_token, "access.jwt")
        assert.eq(tokens.id_token, "id.jwt")
        "#,
        public.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_hydra_accept_login() {
    let admin = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/admin/oauth2/auth/requests/login/accept"))
        .and(query_param("login_challenge", "abc123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "redirect_to": "https://hydra.example.com/oauth2/auth?client_id=cc&..."
        })))
        .mount(&admin)
        .await;

    let script = format!(
        r#"
        local hydra = require("assay.hydra")
        local h = hydra.client({{ admin_url = "{}" }})
        local result = h:accept_login("abc123", "user:alice", {{ remember = true, remember_for = 86400 }})
        assert.contains(result.redirect_to, "hydra.example.com")
        "#,
        admin.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_hydra_accept_consent_with_claims() {
    let admin = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/admin/oauth2/auth/requests/consent/accept"))
        .and(query_param("consent_challenge", "xyz789"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "redirect_to": "https://cc.example.com/auth/callback?code=..."
        })))
        .mount(&admin)
        .await;

    let script = format!(
        r#"
        local hydra = require("assay.hydra")
        local h = hydra.client({{ admin_url = "{}" }})
        local result = h:accept_consent("xyz789", {{
          grant_scope = {{"openid", "profile", "email"}},
          session = {{
            id_token = {{
              sub = "user:alice",
              email = "alice@example.com",
              role = "admin",
            }},
          }},
        }})
        assert.contains(result.redirect_to, "cc.example.com")
        "#,
        admin.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_hydra_get_logout_request() {
    let admin = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/admin/oauth2/auth/requests/logout"))
        .and(query_param("logout_challenge", "logout-abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "request_url": "https://hydra.example.com/oauth2/sessions/logout",
            "rp_initiated": true,
            "sid": "session-xyz",
            "subject": "user:alice",
            "client": { "client_id": "command-center" }
        })))
        .mount(&admin)
        .await;

    let script = format!(
        r#"
        local hydra = require("assay.hydra")
        local h = hydra.client({{ admin_url = "{}" }})
        local req = h:get_logout_request("logout-abc")
        assert.eq(req.subject, "user:alice")
        assert.eq(req.rp_initiated, true)
        assert.eq(req.client.client_id, "command-center")
        "#,
        admin.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_hydra_accept_logout() {
    let admin = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/admin/oauth2/auth/requests/logout/accept"))
        .and(query_param("logout_challenge", "logout-abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "redirect_to": "https://command-center.example.com/auth/login"
        })))
        .mount(&admin)
        .await;

    let script = format!(
        r#"
        local hydra = require("assay.hydra")
        local h = hydra.client({{ admin_url = "{}" }})
        local result = h:accept_logout("logout-abc")
        assert.contains(result.redirect_to, "command-center.example.com")
        "#,
        admin.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_hydra_reject_logout() {
    let admin = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/admin/oauth2/auth/requests/logout/reject"))
        .and(query_param("logout_challenge", "logout-abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&admin)
        .await;

    let script = format!(
        r#"
        local hydra = require("assay.hydra")
        local h = hydra.client({{ admin_url = "{}" }})
        h:reject_logout("logout-abc")
        "#,
        admin.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_hydra_introspect() {
    let admin = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/admin/oauth2/introspect"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "active": true,
            "sub": "user:alice",
            "scope": "openid profile email"
        })))
        .mount(&admin)
        .await;

    let script = format!(
        r#"
        local hydra = require("assay.hydra")
        local h = hydra.client({{ admin_url = "{}" }})
        local info = h:introspect("access.jwt")
        assert.eq(info.active, true)
        assert.eq(info.sub, "user:alice")
        "#,
        admin.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_hydra_well_known() {
    let public = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": "https://hydra.example.com",
            "authorization_endpoint": "https://hydra.example.com/oauth2/auth",
            "token_endpoint": "https://hydra.example.com/oauth2/token"
        })))
        .mount(&public)
        .await;

    let script = format!(
        r#"
        local hydra = require("assay.hydra")
        local h = hydra.client({{ public_url = "{}" }})
        local wk = h:well_known()
        assert.contains(wk.issuer, "hydra.example.com")
        "#,
        public.uri()
    );
    run_lua(&script).await.unwrap();
}
