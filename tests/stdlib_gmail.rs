mod common;

use common::run_lua;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_gmail() {
    let script = r#"
        local mod = require("assay.gmail")
        assert.not_nil(mod)
        assert.not_nil(mod.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_gmail_search() {
    let server = MockServer::start().await;

    // Mock the message list endpoint
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "messages": [
                { "id": "msg-1", "threadId": "thread-1" },
                { "id": "msg-2", "threadId": "thread-2" }
            ]
        })))
        .mount(&server)
        .await;

    // Mock individual message fetches
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/messages/msg-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "msg-1",
            "threadId": "thread-1",
            "snippet": "Hello from test",
            "payload": {
                "headers": [
                    { "name": "From", "value": "alice@example.com" },
                    { "name": "Subject", "value": "Test email 1" }
                ]
            }
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/messages/msg-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "msg-2",
            "threadId": "thread-2",
            "snippet": "Another test",
            "payload": {
                "headers": [
                    { "name": "From", "value": "bob@example.com" },
                    { "name": "Subject", "value": "Test email 2" }
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gmail = require("assay.gmail")
        local tmpdir = fs.tempdir()

        -- Write mock credentials
        fs.write(tmpdir .. "/creds.json", json.encode({{
            client_id = "test-id",
            client_secret = "test-secret",
        }}))
        fs.write(tmpdir .. "/token.json", json.encode({{
            access_token = "valid-token",
            refresh_token = "refresh-tok",
        }}))

        local c = gmail.client({{
            credentials_file = tmpdir .. "/creds.json",
            token_file = tmpdir .. "/token.json",
            api_base = "{}",
        }})
        local messages = c.messages:search("from:alice")
        assert.eq(#messages, 2)
        assert.eq(messages[1].id, "msg-1")
        assert.eq(messages[1].snippet, "Hello from test")
        assert.eq(messages[2].id, "msg-2")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gmail_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/messages/msg-abc"))
        .and(query_param("format", "full"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "msg-abc",
            "threadId": "thread-1",
            "snippet": "Important message",
            "payload": {
                "mimeType": "text/plain",
                "body": { "data": "SGVsbG8gV29ybGQ=" },
                "headers": [
                    { "name": "Subject", "value": "Important" }
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gmail = require("assay.gmail")
        local tmpdir = fs.tempdir()
        fs.write(tmpdir .. "/creds.json", json.encode({{
            client_id = "id", client_secret = "secret",
        }}))
        fs.write(tmpdir .. "/token.json", json.encode({{
            access_token = "tok", refresh_token = "ref",
        }}))
        local c = gmail.client({{
            credentials_file = tmpdir .. "/creds.json",
            token_file = tmpdir .. "/token.json",
            api_base = "{}",
        }})
        local msg = c.messages:get("msg-abc")
        assert.eq(msg.id, "msg-abc")
        assert.eq(msg.snippet, "Important message")
        assert.eq(msg.payload.mimeType, "text/plain")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gmail_send() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/gmail/v1/users/me/messages/send"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "sent-msg-1",
            "threadId": "new-thread",
            "labelIds": ["SENT"]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gmail = require("assay.gmail")
        local tmpdir = fs.tempdir()
        fs.write(tmpdir .. "/creds.json", json.encode({{
            client_id = "id", client_secret = "secret",
        }}))
        fs.write(tmpdir .. "/token.json", json.encode({{
            access_token = "tok", refresh_token = "ref",
        }}))
        local c = gmail.client({{
            credentials_file = tmpdir .. "/creds.json",
            token_file = tmpdir .. "/token.json",
            api_base = "{}",
        }})
        local result = c.messages:send("user@example.com", "Test Subject", "Hello there!")
        assert.eq(result.id, "sent-msg-1")
        assert.eq(result.labelIds[1], "SENT")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gmail_reply() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/messages/msg-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "msg-1",
            "threadId": "thread-1",
            "payload": {
                "headers": [
                    { "name": "From", "value": "alice@example.com" },
                    { "name": "Subject", "value": "Status update" },
                    { "name": "Message-Id", "value": "<msg-1@example.com>" }
                ]
            }
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/gmail/v1/users/me/messages/send"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "reply-1",
            "threadId": "thread-1"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gmail = require("assay.gmail")
        local tmpdir = fs.tempdir()
        fs.write(tmpdir .. "/creds.json", json.encode({{
            client_id = "id", client_secret = "secret",
        }}))
        fs.write(tmpdir .. "/token.json", json.encode({{
            access_token = "tok", refresh_token = "ref",
        }}))
        local c = gmail.client({{
            credentials_file = tmpdir .. "/creds.json",
            token_file = tmpdir .. "/token.json",
            api_base = "{}",
        }})
        local result = c.messages:reply("msg-1", {{ body = "Thanks!" }})
        assert.eq(result.id, "reply-1")
        assert.eq(result.threadId, "thread-1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gmail_labels() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/labels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "labels": [
                { "id": "INBOX", "name": "INBOX", "type": "system" },
                { "id": "SENT", "name": "SENT", "type": "system" },
                { "id": "Label_1", "name": "Work", "type": "user" }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gmail = require("assay.gmail")
        local tmpdir = fs.tempdir()
        fs.write(tmpdir .. "/creds.json", json.encode({{
            client_id = "id", client_secret = "secret",
        }}))
        fs.write(tmpdir .. "/token.json", json.encode({{
            access_token = "tok", refresh_token = "ref",
        }}))
        local c = gmail.client({{
            credentials_file = tmpdir .. "/creds.json",
            token_file = tmpdir .. "/token.json",
            api_base = "{}",
        }})
        local labels = c.labels:list()
        assert.eq(#labels, 3)
        assert.eq(labels[1].name, "INBOX")
        assert.eq(labels[3].name, "Work")
        assert.eq(labels[3].type, "user")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gmail_token_refresh() {
    let server = MockServer::start().await;

    // First call returns 401
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/labels"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": { "code": 401, "message": "Invalid Credentials" }
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // Token refresh endpoint
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "new-access-token",
            "token_type": "Bearer",
            "expires_in": 3600
        })))
        .mount(&server)
        .await;

    // Second call with new token succeeds
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/labels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "labels": [
                { "id": "INBOX", "name": "INBOX", "type": "system" }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gmail = require("assay.gmail")
        local tmpdir = fs.tempdir()
        fs.write(tmpdir .. "/creds.json", json.encode({{
            client_id = "id", client_secret = "secret",
        }}))
        fs.write(tmpdir .. "/token.json", json.encode({{
            access_token = "expired-token", refresh_token = "refresh-tok",
        }}))
        local c = gmail.client({{
            credentials_file = tmpdir .. "/creds.json",
            token_file = tmpdir .. "/token.json",
            api_base = "{}",
            token_url = "{}/token",
        }})
        local labels = c.labels:list()
        assert.eq(#labels, 1)
        assert.eq(labels[1].name, "INBOX")
        "#,
        server.uri(),
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
