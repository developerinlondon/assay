mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_gcal() {
    let script = r#"
        local mod = require("assay.gcal")
        assert.not_nil(mod)
        assert.not_nil(mod.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_gcal_events() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/calendars/primary/events"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "calendar#events",
            "items": [
                {
                    "id": "evt-1",
                    "summary": "Team standup",
                    "start": { "dateTime": "2026-04-05T09:00:00Z" },
                    "end": { "dateTime": "2026-04-05T09:30:00Z" }
                },
                {
                    "id": "evt-2",
                    "summary": "Lunch",
                    "start": { "dateTime": "2026-04-05T12:00:00Z" },
                    "end": { "dateTime": "2026-04-05T13:00:00Z" }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gcal = require("assay.gcal")
        local tmpdir = fs.tempdir()
        fs.write(tmpdir .. "/creds.json", json.encode({{
            client_id = "id", client_secret = "secret",
        }}))
        fs.write(tmpdir .. "/token.json", json.encode({{
            access_token = "tok", refresh_token = "ref",
        }}))
        local c = gcal.client({{
            credentials_file = tmpdir .. "/creds.json",
            token_file = tmpdir .. "/token.json",
            api_base = "{}",
        }})
        local events = c:events()
        assert.eq(#events, 2)
        assert.eq(events[1].id, "evt-1")
        assert.eq(events[1].summary, "Team standup")
        assert.eq(events[2].summary, "Lunch")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gcal_event_create() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/calendar/v3/calendars/primary/events"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "new-evt",
            "summary": "New Meeting",
            "status": "confirmed",
            "start": { "dateTime": "2026-04-06T10:00:00Z" },
            "end": { "dateTime": "2026-04-06T11:00:00Z" }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gcal = require("assay.gcal")
        local tmpdir = fs.tempdir()
        fs.write(tmpdir .. "/creds.json", json.encode({{
            client_id = "id", client_secret = "secret",
        }}))
        fs.write(tmpdir .. "/token.json", json.encode({{
            access_token = "tok", refresh_token = "ref",
        }}))
        local c = gcal.client({{
            credentials_file = tmpdir .. "/creds.json",
            token_file = tmpdir .. "/token.json",
            api_base = "{}",
        }})
        local evt = c:event_create({{
            summary = "New Meeting",
            start = {{ dateTime = "2026-04-06T10:00:00Z" }},
            ["end"] = {{ dateTime = "2026-04-06T11:00:00Z" }},
        }})
        assert.eq(evt.id, "new-evt")
        assert.eq(evt.summary, "New Meeting")
        assert.eq(evt.status, "confirmed")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gcal_event_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/calendars/primary/events/evt-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "evt-123",
            "summary": "Sprint Planning",
            "description": "Q2 planning session",
            "status": "confirmed",
            "start": { "dateTime": "2026-04-07T14:00:00Z" },
            "end": { "dateTime": "2026-04-07T15:00:00Z" },
            "attendees": [
                { "email": "alice@example.com", "responseStatus": "accepted" }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gcal = require("assay.gcal")
        local tmpdir = fs.tempdir()
        fs.write(tmpdir .. "/creds.json", json.encode({{
            client_id = "id", client_secret = "secret",
        }}))
        fs.write(tmpdir .. "/token.json", json.encode({{
            access_token = "tok", refresh_token = "ref",
        }}))
        local c = gcal.client({{
            credentials_file = tmpdir .. "/creds.json",
            token_file = tmpdir .. "/token.json",
            api_base = "{}",
        }})
        local evt = c:event_get("evt-123")
        assert.eq(evt.id, "evt-123")
        assert.eq(evt.summary, "Sprint Planning")
        assert.eq(evt.description, "Q2 planning session")
        assert.eq(#evt.attendees, 1)
        assert.eq(evt.attendees[1].email, "alice@example.com")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gcal_event_update() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/calendar/v3/calendars/primary/events/evt-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "evt-123",
            "summary": "Updated Planning",
            "status": "confirmed"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gcal = require("assay.gcal")
        local tmpdir = fs.tempdir()
        fs.write(tmpdir .. "/creds.json", json.encode({{
            client_id = "id", client_secret = "secret",
        }}))
        fs.write(tmpdir .. "/token.json", json.encode({{
            access_token = "tok", refresh_token = "ref",
        }}))
        local c = gcal.client({{
            credentials_file = tmpdir .. "/creds.json",
            token_file = tmpdir .. "/token.json",
            api_base = "{}",
        }})
        local evt = c:event_update("evt-123", {{ summary = "Updated Planning" }})
        assert.eq(evt.id, "evt-123")
        assert.eq(evt.summary, "Updated Planning")
        assert.eq(evt.status, "confirmed")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gcal_calendars() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/users/me/calendarList"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "calendar#calendarList",
            "items": [
                { "id": "primary", "summary": "My Calendar", "primary": true },
                { "id": "holidays", "summary": "UK Holidays", "primary": false }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gcal = require("assay.gcal")
        local tmpdir = fs.tempdir()
        fs.write(tmpdir .. "/creds.json", json.encode({{
            client_id = "id", client_secret = "secret",
        }}))
        fs.write(tmpdir .. "/token.json", json.encode({{
            access_token = "tok", refresh_token = "ref",
        }}))
        local c = gcal.client({{
            credentials_file = tmpdir .. "/creds.json",
            token_file = tmpdir .. "/token.json",
            api_base = "{}",
        }})
        local cals = c:calendars()
        assert.eq(#cals, 2)
        assert.eq(cals[1].summary, "My Calendar")
        assert.eq(cals[1].primary, true)
        assert.eq(cals[2].summary, "UK Holidays")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gcal_event_delete() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/calendar/v3/calendars/primary/events/evt-del"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gcal = require("assay.gcal")
        local tmpdir = fs.tempdir()
        fs.write(tmpdir .. "/creds.json", json.encode({{
            client_id = "id", client_secret = "secret",
        }}))
        fs.write(tmpdir .. "/token.json", json.encode({{
            access_token = "tok", refresh_token = "ref",
        }}))
        local c = gcal.client({{
            credentials_file = tmpdir .. "/creds.json",
            token_file = tmpdir .. "/token.json",
            api_base = "{}",
        }})
        local result = c:event_delete("evt-del")
        assert.eq(result, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gcal_token_refresh_on_401() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/calendar/v3/users/me/calendarList"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": { "code": 401, "message": "Invalid Credentials" }
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "new-access-token"
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/calendar/v3/users/me/calendarList"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [
                { "id": "primary", "summary": "My Calendar", "primary": true }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gcal = require("assay.gcal")
        local tmpdir = fs.tempdir()
        fs.write(tmpdir .. "/creds.json", json.encode({{
            client_id = "id", client_secret = "secret",
        }}))
        fs.write(tmpdir .. "/token.json", json.encode({{
            access_token = "expired-token", refresh_token = "refresh-tok",
        }}))
        local c = gcal.client({{
            credentials_file = tmpdir .. "/creds.json",
            token_file = tmpdir .. "/token.json",
            api_base = "{}",
            token_url = "{}/token",
        }})
        local cals = c:calendars()
        assert.eq(#cals, 1)
        assert.eq(cals[1].summary, "My Calendar")
        "#,
        server.uri(),
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
