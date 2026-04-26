mod common;

use common::run_lua;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn release_body(server_uri: &str) -> serde_json::Value {
    json!({
        "tag_name": "v1.2.3",
        "name": "Release 1.2.3",
        "published_at": "2024-01-01T00:00:00Z",
        "assets": [
            {
                "name": "tool-x86_64-unknown-linux-musl.tar.gz",
                "browser_download_url": format!("{server_uri}/dl/tool.tar.gz"),
                "size": 100
            },
            {
                "name": "tool-x86_64-unknown-linux-musl.tar.gz.sha256",
                "browser_download_url": format!("{server_uri}/dl/tool.tar.gz.sha256"),
                "size": 96
            }
        ]
    })
}

#[tokio::test]
async fn test_latest_release_basic() {
    let server = MockServer::start().await;
    let body = release_body(&server.uri());
    Mock::given(method("GET"))
        .and(path("/repos/octo/tool/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local github = require("assay.github")
        local rel = github.latest_release("octo", "tool", {{ base_url = "{}" }})
        assert.eq(rel.tag_name, "v1.2.3")
        assert.eq(rel.version, "1.2.3")
        assert.eq(#rel.assets, 2)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_find_asset_by_pattern() {
    let server = MockServer::start().await;
    let body = release_body(&server.uri());
    Mock::given(method("GET"))
        .and(path("/repos/octo/tool/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local github = require("assay.github")
        local rel = github.latest_release("octo", "tool", {{ base_url = "{}" }})
        local sha = github.find_asset(rel, "tar%.gz%.sha256$")
        assert.not_nil(sha)
        assert.eq(sha.name, "tool-x86_64-unknown-linux-musl.tar.gz.sha256")

        local missing = github.find_asset(rel, "%.zip$")
        assert.eq(missing, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_release_checksum() {
    let server = MockServer::start().await;
    let body = release_body(&server.uri());
    Mock::given(method("GET"))
        .and(path("/repos/octo/tool/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/dl/tool.tar.gz.sha256"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "deadbeefcafebabe0123456789abcdef0123456789abcdef0123456789abcdef  tool-x86_64-unknown-linux-musl.tar.gz\n",
        ))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local github = require("assay.github")
        local rel = github.latest_release("octo", "tool", {{ base_url = "{}" }})
        local hex = github.release_checksum(rel, {{
            asset_pattern = "tar%.gz$",
            digest = "sha256",
        }})
        assert.eq(hex, "deadbeefcafebabe0123456789abcdef0123456789abcdef0123456789abcdef")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_fetch_asset_text() {
    let server = MockServer::start().await;
    let body = release_body(&server.uri());
    Mock::given(method("GET"))
        .and(path("/repos/octo/tool/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/dl/tool.tar.gz.sha256"))
        .respond_with(ResponseTemplate::new(200).set_body_string("abc123  tool.tar.gz\n"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local github = require("assay.github")
        local rel = github.latest_release("octo", "tool", {{ base_url = "{}" }})
        local asset = github.find_asset(rel, "%.sha256$")
        local body = github.fetch_asset_text(asset)
        assert.eq(body, "abc123  tool.tar.gz\n")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
