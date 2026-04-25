//! assay.auth Lua wrapper tests.
//!
//! Each test spins up a wiremock server, points the auth client at it,
//! and exercises one binding. Mirrors the shape of the existing
//! `stdlib_ory_kratos.rs` / `stdlib_dex.rs` tests.

mod common;

use common::run_lua;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_auth_require() {
    let script = r#"
        local auth = require("assay.auth")
        assert.not_nil(auth)
        assert.not_nil(auth.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_auth_login_returns_session_payload() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "user_id": "usr_test",
            "email": "alice@example.com",
            "csrf_token": "csrf_abc",
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local auth = require("assay.auth")
        local c = auth.client({{ engine_url = "{}" }})
        local r = c:login("alice@example.com", "secret")
        assert.eq(r.user_id, "usr_test")
        assert.eq(r.csrf_token, "csrf_abc")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_auth_whoami_returns_user_when_authenticated() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth/whoami"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "user_id": "usr_test",
            "email": "alice@example.com",
            "email_verified": true,
            "display_name": "Alice",
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local auth = require("assay.auth")
        local c = auth.client({{ engine_url = "{}" }})
        local who = c:whoami()
        assert.not_nil(who)
        assert.eq(who.email, "alice@example.com")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_auth_whoami_returns_nil_when_unauthenticated() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth/whoami"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": "no session"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local auth = require("assay.auth")
        local c = auth.client({{ engine_url = "{}" }})
        local who = c:whoami()
        assert.eq(who, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_auth_biscuit_public_pem() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth/admin/auth/biscuit"))
        .and(header("authorization", "Bearer adm-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kid": "kid_test",
            "public_pem": "-----BEGIN PUBLIC KEY-----\nABC\n-----END PUBLIC KEY-----\n",
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local auth = require("assay.auth")
        local c = auth.client({{ engine_url = "{}", api_key = "adm-token" }})
        local pem = c.biscuit:public_pem()
        assert.contains(pem, "BEGIN PUBLIC KEY")
        local kid = c.biscuit:active_kid()
        assert.eq(kid, "kid_test")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_auth_zanzibar_check_allowed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/admin/auth/zanzibar/check"))
        .and(header("authorization", "Bearer adm-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": "Allowed",
            "allowed": true,
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local auth = require("assay.auth")
        local c = auth.client({{ engine_url = "{}", api_key = "adm-token" }})
        local ok, r = c.zanzibar:check("document", "readme", "view", "user", "alice")
        assert.eq(ok, true)
        assert.eq(r.result, "Allowed")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_auth_zanzibar_check_denied() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/admin/auth/zanzibar/check"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": "Denied",
            "allowed": false,
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local auth = require("assay.auth")
        local c = auth.client({{ engine_url = "{}", api_key = "adm" }})
        local ok, r = c.zanzibar:check("document", "secret", "view", "user", "eve")
        assert.eq(ok, false)
        assert.eq(r.result, "Denied")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_auth_users_list_with_admin_key() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth/admin/auth/users"))
        .and(header("authorization", "Bearer adm-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [
                { "id": "usr_1", "email": "a@x.com", "email_verified": true, "display_name": "A", "created_at": 1.0 },
                { "id": "usr_2", "email": "b@x.com", "email_verified": false, "display_name": null, "created_at": 2.0 }
            ],
            "total": 2,
            "limit": 50,
            "offset": 0
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local auth = require("assay.auth")
        local c = auth.client({{ engine_url = "{}", api_key = "adm-token" }})
        local r = c.users:list()
        assert.eq(r.total, 2)
        assert.eq(#r.items, 2)
        assert.eq(r.items[1].email, "a@x.com")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_auth_users_create() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/admin/auth/users"))
        .and(header("authorization", "Bearer adm"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "usr_new",
            "email": "new@example.com",
            "email_verified": false,
            "display_name": "New",
            "created_at": 999.0
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local auth = require("assay.auth")
        local c = auth.client({{ engine_url = "{}", api_key = "adm" }})
        local u = c.users:create({{ email = "new@example.com", display_name = "New" }})
        assert.eq(u.id, "usr_new")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_auth_sessions_list_for_user() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth/admin/auth/sessions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [
                { "id": "sess_1", "user_id": "usr_1", "csrf_token": "x", "created_at": 1.0, "expires_at": 100.0, "ip_hash": null, "user_agent_hash": null }
            ],
            "total": 1,
            "limit": 50,
            "offset": 0
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local auth = require("assay.auth")
        local c = auth.client({{ engine_url = "{}", api_key = "adm" }})
        local r = c.sessions:list_for_user("usr_1")
        assert.eq(r.total, 1)
        assert.eq(r.items[1].id, "sess_1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_auth_oidc_clients_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth/admin/oidc/clients"))
        .and(header("authorization", "Bearer adm"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "client_id": "ocl_1",
                "client_secret_hash": "$argon2id$xyz",
                "redirect_uris": ["https://app.example/cb"],
                "name": "App",
                "logo_url": null,
                "token_endpoint_auth_method": "client_secret_basic",
                "default_scopes": ["openid"],
                "require_consent": true,
                "grant_types": ["authorization_code"],
                "response_types": ["code"],
                "pkce_required": true,
                "backchannel_logout_uri": null,
                "created_at": 1.0
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local auth = require("assay.auth")
        local c = auth.client({{ engine_url = "{}", api_key = "adm" }})
        local list = c.oidc_clients:list()
        assert.eq(#list, 1)
        assert.eq(list[1].client_id, "ocl_1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_auth_admin_request_without_key_fails() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth/admin/auth/users"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": "admin disabled — no admin_api_keys configured"
        })))
        .mount(&server)
        .await;

    // No api_key supplied — engine returns 401, Lua wrapper raises.
    let script = format!(
        r#"
        local auth = require("assay.auth")
        local c = auth.client({{ engine_url = "{}" }})
        local ok, err = pcall(function() c.users:list() end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "401")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
