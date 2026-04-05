mod common;

use common::run_lua;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_oauth2() {
    let script = r#"
        local mod = require("assay.oauth2")
        assert.not_nil(mod)
        assert.not_nil(mod.from_file)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_oauth2_from_file() {
    let script = r#"
        local oauth2 = require("assay.oauth2")
        local tmpdir = fs.tempdir()
        fs.write(tmpdir .. "/credentials.json", json.encode({
            installed = {
                client_id = "test-client",
                client_secret = "test-secret",
            }
        }))
        fs.write(tmpdir .. "/token.json", json.encode({
            access_token = "token-123",
            refresh_token = "refresh-123",
        }))

        local client = oauth2.from_file(tmpdir .. "/credentials.json", tmpdir .. "/token.json")
        assert.eq(client._credentials.client_id, "test-client")
        assert.eq(client._token_data.refresh_token, "refresh-123")
        assert.eq(client._token_file, tmpdir .. "/token.json")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_oauth2_access_token() {
    let script = r#"
        local oauth2 = require("assay.oauth2")
        local tmpdir = fs.tempdir()
        fs.write(tmpdir .. "/credentials.json", json.encode({
            client_id = "test-client",
            client_secret = "test-secret",
        }))
        fs.write(tmpdir .. "/token.json", json.encode({
            access_token = "token-abc",
            refresh_token = "refresh-abc",
        }))

        local client = oauth2.from_file(tmpdir .. "/credentials.json", tmpdir .. "/token.json")
        assert.eq(client:access_token(), "token-abc")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_oauth2_refresh() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_json(serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": "refresh-xyz",
            "client_id": "test-client",
            "client_secret": "test-secret"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "fresh-token",
            "refresh_token": "refresh-next"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local oauth2 = require("assay.oauth2")
        local tmpdir = fs.tempdir()
        fs.write(tmpdir .. "/credentials.json", json.encode({{
            client_id = "test-client",
            client_secret = "test-secret",
        }}))
        fs.write(tmpdir .. "/token.json", json.encode({{
            access_token = "stale-token",
            refresh_token = "refresh-xyz",
        }}))

        local client = oauth2.from_file(tmpdir .. "/credentials.json", tmpdir .. "/token.json", {{
            token_url = "{}/token",
        }})
        local refreshed = client:refresh()
        assert.eq(refreshed, "fresh-token")
        assert.eq(client:access_token(), "fresh-token")
        assert.eq(client._token_data.refresh_token, "refresh-next")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_oauth2_refresh_401() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": "invalid_grant"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local oauth2 = require("assay.oauth2")
        local tmpdir = fs.tempdir()
        fs.write(tmpdir .. "/credentials.json", json.encode({{
            client_id = "test-client",
            client_secret = "test-secret",
        }}))
        fs.write(tmpdir .. "/token.json", json.encode({{
            access_token = "stale-token",
            refresh_token = "refresh-xyz",
        }}))

        local client = oauth2.from_file(tmpdir .. "/credentials.json", tmpdir .. "/token.json", {{
            token_url = "{}/token",
        }})
        local ok, err = pcall(function()
            client:refresh()
        end)
        assert.eq(ok, false)
        assert.contains(err, "oauth2: token refresh failed HTTP 401")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_oauth2_headers() {
    let script = r#"
        local oauth2 = require("assay.oauth2")
        local tmpdir = fs.tempdir()
        fs.write(tmpdir .. "/credentials.json", json.encode({
            client_id = "test-client",
            client_secret = "test-secret",
        }))
        fs.write(tmpdir .. "/token.json", json.encode({
            access_token = "header-token",
            refresh_token = "refresh-abc",
        }))

        local client = oauth2.from_file(tmpdir .. "/credentials.json", tmpdir .. "/token.json")
        local headers = client:headers()
        assert.eq(headers["Authorization"], "Bearer header-token")
        assert.eq(headers["Content-Type"], "application/json")
    "#;
    run_lua(script).await.unwrap();
}
