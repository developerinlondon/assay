//! Tests for the assay-engine vault Lua client (`assay.vault`).
//!
//! Mocks the engine's `/api/v1/vault/*` HTTP surface with wiremock and
//! exercises the Lua client end-to-end. Covers the Phase-1 surface
//! (KV v2 lifecycle + transit encrypt / decrypt / rotate). The real
//! crypto path is covered by the `assay-vault` crate's own integration
//! tests; here we just verify the Lua wrapper hits the right URLs and
//! parses the right shapes.

mod common;

use common::run_lua;
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ADMIN_KEY: &str = "test-admin-key";

fn auth_header() -> impl wiremock::Match {
    header("Authorization", &format!("Bearer {ADMIN_KEY}")[..])
}

#[tokio::test]
async fn kv_put_then_get_round_trip() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path("/api/v1/vault/kv/api/stripe"))
        .and(auth_header())
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "path": "api/stripe",
            "version": 1,
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/vault/kv/api/stripe"))
        .and(auth_header())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "path": "api/stripe",
            "version": 1,
            "data": "sk_live_xxx",
            "deleted_at": null,
            "created_at": 1700000000.0,
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client({{ engine_url = "{base}", admin_key = "{key}" }})
        local put = c.kv:put("api/stripe", "sk_live_xxx")
        assert.eq(put.version, 1)
        local got = c.kv:get("api/stripe")
        assert.eq(got.data, "sk_live_xxx")
        assert.eq(got.version, 1)
        "#,
        base = server.uri(),
        key = ADMIN_KEY,
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn kv_get_specific_version_query() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/vault/kv/k"))
        .and(query_param("version", "3"))
        .and(auth_header())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "path": "k",
            "version": 3,
            "data": "v3",
            "deleted_at": null,
            "created_at": 1700000000.0,
        })))
        .mount(&server)
        .await;
    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client({{ engine_url = "{base}", admin_key = "{key}" }})
        local got = c.kv:get("k", 3)
        assert.eq(got.data, "v3")
        "#,
        base = server.uri(),
        key = ADMIN_KEY,
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn kv_lifecycle_paths() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/vault/kv/k"))
        .and(query_param("version", "2"))
        .and(auth_header())
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/vault/kv-destroy/k"))
        .and(query_param("version", "2"))
        .and(auth_header())
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/vault/kv-undelete/k"))
        .and(query_param("version", "2"))
        .and(auth_header())
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client({{ engine_url = "{base}", admin_key = "{key}" }})
        c.kv:delete("k", 2)
        c.kv:destroy("k", 2)
        c.kv:undelete("k", 2)
        "#,
        base = server.uri(),
        key = ADMIN_KEY,
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn kv_list_with_and_without_prefix() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/vault/kv-list/api/"))
        .and(auth_header())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entries": [
                {"path": "api/stripe", "latest_version": 1, "custom_md": {}, "created_at": 0.0, "updated_at": 0.0},
            ],
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/vault/kv-list"))
        .and(auth_header())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entries": [],
        })))
        .mount(&server)
        .await;
    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client({{ engine_url = "{base}", admin_key = "{key}" }})
        local under_api = c.kv:list("api/")
        assert.eq(#under_api.entries, 1)
        local everything = c.kv:list()
        assert.eq(#everything.entries, 0)
        "#,
        base = server.uri(),
        key = ADMIN_KEY,
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn transit_create_encrypt_decrypt_rotate() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/vault/transit/keys/logs"))
        .and(auth_header())
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/vault/transit/encrypt/logs"))
        .and(auth_header())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ciphertext": "vault:v1:abcdef==",
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/vault/transit/decrypt/logs"))
        .and(auth_header())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            // base64 of "hello"
            "plaintext_b64": "aGVsbG8=",
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/vault/transit/keys/logs/rotate"))
        .and(auth_header())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "logs",
            "version": 2,
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client({{ engine_url = "{base}", admin_key = "{key}" }})
        c.transit:create("logs")
        local ct = c.transit:encrypt("logs", "anything")
        assert.eq(ct, "vault:v1:abcdef==")
        local pt = c.transit:decrypt("logs", ct)
        assert.eq(pt, "hello")
        local r = c.transit:rotate("logs")
        assert.eq(r.version, 2)
        "#,
        base = server.uri(),
        key = ADMIN_KEY,
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn missing_engine_url_errors() {
    let script = r#"
        local vault = require("assay.vault")
        local ok, err = pcall(function() return vault.client({}) end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "engine_url")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn unauth_response_surfaces_as_lua_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/vault/kv/x"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": "unauthorized",
            "error_description": "missing or invalid Bearer token",
        })))
        .mount(&server)
        .await;
    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client({{ engine_url = "{base}", admin_key = "" }})
        local ok, err = pcall(function() return c.kv:get("x") end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "401")
        "#,
        base = server.uri(),
    );
    run_lua(&script).await.unwrap();
}
