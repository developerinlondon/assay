//! Fixtures shared by the `assay.excalidash` test binaries.
//!
//! Response shapes here are transcribed from a live ExcaliDash backend, so a
//! test that passes against them is asserting against the real wire format
//! rather than an invented one.

#![allow(dead_code)]

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub const KEY: &str = "exd_abc123def456_secretsecretsecret";
pub const JWT: &str = "eyJhbGciOiJIUzI1NiJ9.session.token";
pub const CSRF_COOKIE: &str = "excalidash-csrf-client";
pub const CSRF_TOKEN: &str = "eyJ0cyI6MX0.csrfsignature";
pub const CSRF_COOKIE_VALUE: &str = "cookievalue123456";

/// A drawing summary as `GET /drawings` reports it: no scene, and the owner's
/// display name flattened onto the row.
pub fn summary(id: &str, name: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id, "name": name, "collectionId": serde_json::Value::Null, "version": 1,
        "createdAt": "2026-08-13T15:55:27.177Z", "updatedAt": "2026-08-13T15:55:27.177Z",
        "creatorName": "Probe",
    })
}

pub fn drawing(id: &str, name: &str, version: i64) -> serde_json::Value {
    serde_json::json!({
        "id": id, "name": name,
        "elements": [{ "id": "el1", "type": "rectangle", "x": 10, "y": 20 }],
        "appState": { "viewBackgroundColor": "#ffffff" },
        "files": {}, "preview": serde_json::Value::Null, "version": version,
        "userId": "user-1", "collectionId": serde_json::Value::Null,
        "createdAt": "2026-08-13T15:55:27.177Z", "updatedAt": "2026-08-13T15:55:27.177Z",
        "accessLevel": "owner",
    })
}

pub fn page(rows: serde_json::Value, total: i64) -> serde_json::Value {
    serde_json::json!({ "drawings": rows, "totalCount": total })
}

pub fn ok() -> serde_json::Value {
    serde_json::json!({ "success": true })
}

/// Mount one route. Everything the module talks to is JSON over a fixed status,
/// so a test says what a route answers rather than how to wire a mock.
pub async fn mount(
    server: &MockServer,
    verb: &str,
    route: &str,
    status: u16,
    body: serde_json::Value,
) {
    Mock::given(method(verb))
        .and(path(format!("/api{route}")))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

pub async fn mount_ok(server: &MockServer, verb: &str, route: &str, body: serde_json::Value) {
    mount(server, verb, route, 200, body).await;
}

/// `GET /drawings` answering one page of summaries — the setup most drawing and
/// helper tests open with.
pub async fn mount_list(server: &MockServer, rows: serde_json::Value, total: i64) {
    mount_ok(server, "GET", "/drawings", page(rows, total)).await;
}

/// The CSRF handshake a session write depends on: the token in the body, and
/// the client cookie it is bound to in a Set-Cookie header.
pub async fn mount_csrf(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/csrf-token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "token": CSRF_TOKEN, "header": "x-csrf-token" }))
                .append_header(
                    "set-cookie",
                    format!("{CSRF_COOKIE}={CSRF_COOKIE_VALUE}; Path=/; HttpOnly").as_str(),
                ),
        )
        .mount(server)
        .await;
}

/// Every request body the server received on a route, in order, raw.
async fn bodies_to(server: &MockServer, route: &str) -> Vec<Vec<u8>> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == format!("/api{route}"))
        .map(|r| r.body.clone())
        .collect()
}

pub async fn sent_bodies(server: &MockServer, route: &str) -> Vec<serde_json::Value> {
    bodies_to(server, route)
        .await
        .iter()
        .map(|b| serde_json::from_slice(b).unwrap_or(serde_json::Value::Null))
        .collect()
}

/// The body as it went out on the wire, for the cases where the encoding is the
/// thing under test and re-parsing it would hide the answer.
pub async fn sent_raw(server: &MockServer, route: &str) -> String {
    let bodies = bodies_to(server, route).await;
    let first = bodies
        .first()
        .unwrap_or_else(|| panic!("nothing was sent to {route}"));
    String::from_utf8_lossy(first).into_owned()
}

pub async fn count_calls(server: &MockServer, route: &str) -> usize {
    bodies_to(server, route).await.len()
}

pub async fn count_verb(server: &MockServer, verb: wiremock::http::Method) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method == verb)
        .count()
}

/// A client holding only an API key — the credential most automation will use.
pub fn key_script(uri: &str, body: &str) -> String {
    format!(
        r##"
        local excalidash = require("assay.excalidash")
        local c = excalidash.client({{ api_key = "{KEY}", base_url = "{uri}" }})
        {body}
    "##
    )
}

/// A client holding a session token, which reaches history and sharing too.
pub fn session_script(uri: &str, body: &str) -> String {
    format!(
        r##"
        local excalidash = require("assay.excalidash")
        local c = excalidash.client({{ token = "{JWT}", base_url = "{uri}" }})
        {body}
    "##
    )
}
