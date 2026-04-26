mod common;

use common::run_lua;
use serde_json::json;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn token_response(token: &str, expires_in: u64) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "access_token": token,
        "token_type": "Bearer",
        "expires_in": expires_in,
    }))
}

async fn mount_token(server: &MockServer, token: &str, expires_in: u64) {
    Mock::given(method("POST"))
        .and(path("/api/v2/oauth/token"))
        .and(header("Content-Type", "application/x-www-form-urlencoded"))
        .and(body_string_contains("grant_type=client_credentials"))
        .respond_with(token_response(token, expires_in))
        .mount(server)
        .await;
}

#[tokio::test]
async fn test_require_tailscale() {
    let script = r#"
        local mod = require("assay.tailscale")
        assert.not_nil(mod)
        assert.not_nil(mod.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_tailscale_token_exchange_form_encoded() {
    let server = MockServer::start().await;
    mount_token(&server, "tok-abc", 3600).await;

    // Once a token is cached, list_devices should fire and use bearer auth.
    Mock::given(method("GET"))
        .and(path("/api/v2/tailnet/-/devices"))
        .and(header("Authorization", "Bearer tok-abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "devices": [] })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local tailscale = require("assay.tailscale")
        local ts = tailscale.client({{
          client_id = "ci",
          client_secret = "cs",
          base_url = "{}",
        }})
        local devs = ts:list_devices()
        assert.eq(#devs, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_tailscale_token_body_url_encodes_secret() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v2/oauth/token"))
        .and(header("Content-Type", "application/x-www-form-urlencoded"))
        .and(body_string_contains("grant_type=client_credentials"))
        .and(body_string_contains("client_secret=a%26b%3Dc%2Bd%25e"))
        .respond_with(token_response("tok-safe", 3600))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v2/tailnet/-/devices"))
        .and(header("Authorization", "Bearer tok-safe"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "devices": [] })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local tailscale = require("assay.tailscale")
        local ts = tailscale.client({{
          client_id = "ci",
          client_secret = "a&b=c+d%e",
          base_url = "{}",
        }})
        ts:list_devices()
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_tailscale_mint_key() {
    let server = MockServer::start().await;
    mount_token(&server, "tok-mint", 3600).await;

    Mock::given(method("POST"))
        .and(path("/api/v2/tailnet/-/keys"))
        .and(header("Authorization", "Bearer tok-mint"))
        .and(body_string_contains("\"capabilities\""))
        .and(body_string_contains("\"tag:server\""))
        .and(body_string_contains("\"expirySeconds\":600"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "k123",
            "key": "tskey-auth-abc",
            "expires": "2099-01-01T00:00:00Z",
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local tailscale = require("assay.tailscale")
        local ts = tailscale.client({{
          client_id = "ci",
          client_secret = "cs",
          base_url = "{}",
        }})
        local key = ts:mint_key({{
          reusable = false,
          ephemeral = false,
          preauthorized = true,
          tags = {{ "tag:server" }},
          expiry_seconds = 600,
          description = "test mint",
        }})
        assert.eq(key.id, "k123")
        assert.eq(key.key, "tskey-auth-abc")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_tailscale_list_devices_returns_array() {
    let server = MockServer::start().await;
    mount_token(&server, "tok-list", 3600).await;

    Mock::given(method("GET"))
        .and(path("/api/v2/tailnet/-/devices"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "devices": [
                { "id": "d1", "hostname": "alpha", "name": "alpha.tail.ts.net" },
                { "id": "d2", "hostname": "beta",  "name": "beta.tail.ts.net" }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local tailscale = require("assay.tailscale")
        local ts = tailscale.client({{ client_id = "ci", client_secret = "cs", base_url = "{}" }})
        local devs = ts:list_devices()
        assert.eq(#devs, 2)
        assert.eq(devs[1].hostname, "alpha")
        assert.eq(devs[2].id, "d2")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_tailscale_find_device_match_and_miss() {
    let server = MockServer::start().await;
    mount_token(&server, "tok-find", 3600).await;

    Mock::given(method("GET"))
        .and(path("/api/v2/tailnet/-/devices"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "devices": [
                { "id": "d1", "hostname": "alpha", "name": "alpha.tail.ts.net" },
                { "id": "d2", "hostname": "beta",  "name": "beta.tail.ts.net" }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local tailscale = require("assay.tailscale")
        local ts = tailscale.client({{ client_id = "ci", client_secret = "cs", base_url = "{}" }})
        local hit = ts:find_device({{ hostname = "alpha" }})
        assert.not_nil(hit)
        assert.eq(hit.id, "d1")
        local miss = ts:find_device({{ hostname = "nope" }})
        assert.eq(miss, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_tailscale_set_key_expiry_unchanged() {
    let server = MockServer::start().await;
    mount_token(&server, "tok-uc", 3600).await;

    Mock::given(method("GET"))
        .and(path("/api/v2/device/d1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "d1",
            "hostname": "alpha",
            "keyExpiryDisabled": true
        })))
        .mount(&server)
        .await;

    // Note: we deliberately do NOT mount the POST. If the module sends one,
    // wiremock returns 404 and the assay error path will fire — making the
    // test fail loudly.

    let script = format!(
        r#"
        local tailscale = require("assay.tailscale")
        local ts = tailscale.client({{ client_id = "ci", client_secret = "cs", base_url = "{}" }})
        local result = ts:set_key_expiry("d1", {{ disabled = true }})
        assert.eq(result, "unchanged")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_tailscale_set_key_expiry_changed() {
    let server = MockServer::start().await;
    mount_token(&server, "tok-ch", 3600).await;

    Mock::given(method("GET"))
        .and(path("/api/v2/device/d1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "d1",
            "hostname": "alpha",
            "keyExpiryDisabled": false
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/api/v2/device/d1/key"))
        .and(header("Authorization", "Bearer tok-ch"))
        .and(body_string_contains("\"keyExpiryDisabled\":true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local tailscale = require("assay.tailscale")
        local ts = tailscale.client({{ client_id = "ci", client_secret = "cs", base_url = "{}" }})
        local result = ts:set_key_expiry("d1", {{ disabled = true }})
        assert.eq(result, "changed")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_tailscale_set_device_tags() {
    let server = MockServer::start().await;
    mount_token(&server, "tok-tags", 3600).await;

    Mock::given(method("POST"))
        .and(path("/api/v2/device/d1/tags"))
        .and(header("Authorization", "Bearer tok-tags"))
        .and(body_string_contains("\"tag:foo\""))
        .and(body_string_contains("\"tag:bar\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local tailscale = require("assay.tailscale")
        local ts = tailscale.client({{ client_id = "ci", client_secret = "cs", base_url = "{}" }})
        ts:set_device_tags("d1", {{ "tag:foo", "tag:bar" }})
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_tailscale_authorize_device() {
    let server = MockServer::start().await;
    mount_token(&server, "tok-auth", 3600).await;

    Mock::given(method("POST"))
        .and(path("/api/v2/device/d1/authorized"))
        .and(header("Authorization", "Bearer tok-auth"))
        .and(body_string_contains("\"authorized\":true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local tailscale = require("assay.tailscale")
        local ts = tailscale.client({{ client_id = "ci", client_secret = "cs", base_url = "{}" }})
        ts:authorize_device("d1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_tailscale_delete_device() {
    let server = MockServer::start().await;
    mount_token(&server, "tok-del", 3600).await;

    Mock::given(method("DELETE"))
        .and(path("/api/v2/device/d1"))
        .and(header("Authorization", "Bearer tok-del"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local tailscale = require("assay.tailscale")
        local ts = tailscale.client({{ client_id = "ci", client_secret = "cs", base_url = "{}" }})
        ts:delete_device("d1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_tailscale_acl_test() {
    let server = MockServer::start().await;
    mount_token(&server, "tok-acl", 3600).await;

    Mock::given(method("POST"))
        .and(path("/api/v2/tailnet/-/acl/preview"))
        .and(header("Authorization", "Bearer tok-acl"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "matches": [],
            "user": "alice@example.com",
            "type": "user"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local tailscale = require("assay.tailscale")
        local ts = tailscale.client({{ client_id = "ci", client_secret = "cs", base_url = "{}" }})
        local res = ts:acl_test({{ user = "alice@example.com", type = "user" }})
        assert.eq(res.user, "alice@example.com")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_tailscale_token_endpoint_failure_errors() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v2/oauth/token"))
        .respond_with(ResponseTemplate::new(401).set_body_string("nope"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local tailscale = require("assay.tailscale")
        local ts = tailscale.client({{ client_id = "ci", client_secret = "cs", base_url = "{}" }})
        local ok, err = pcall(function() ts:list_devices() end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "tailscale")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
