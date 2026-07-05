mod common;

use common::run_lua;
use wiremock::matchers::{header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_infoblox() {
    let script = r#"
        local infoblox = require("assay.infoblox")
        assert.not_nil(infoblox)
        assert.not_nil(infoblox.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_infoblox_records_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wapi/v2.12/record:a"))
        .and(query_param("name", "host.example.com"))
        .and(header_exists("Authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"_ref": "record:a/ZG5z:host.example.com/default", "name": "host.example.com", "ipv4addr": "10.0.0.5"}
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local infoblox = require("assay.infoblox")
        local c = infoblox.client("{}", {{ user = "admin", password = "infoblox" }})
        local recs = c.records:get("record:a", {{ name = "host.example.com" }})
        assert.eq(#recs, 1)
        assert.eq(recs[1].name, "host.example.com")
        assert.eq(recs[1].ipv4addr, "10.0.0.5")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_infoblox_records_get_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wapi/v2.12/record:a"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local infoblox = require("assay.infoblox")
        local c = infoblox.client("{}", {{ user = "admin", password = "infoblox" }})
        local recs = c.records:get("record:a", {{ name = "missing.example.com" }})
        assert.eq(recs, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_infoblox_network_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wapi/v2.12/network"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"_ref": "network/ZG5z:10.0.0.0/24/default", "network": "10.0.0.0/24"}
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local infoblox = require("assay.infoblox")
        local c = infoblox.client("{}", {{ user = "admin", password = "infoblox" }})
        local nets = c.network:get({{ network = "10.0.0.0/24" }})
        assert.eq(#nets, 1)
        assert.eq(nets[1].network, "10.0.0.0/24")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_infoblox_range_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wapi/v2.12/range"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"_ref": "range/ZG5z:10.0.0.10/10.0.0.20/default", "start_addr": "10.0.0.10", "end_addr": "10.0.0.20"}
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local infoblox = require("assay.infoblox")
        local c = infoblox.client("{}", {{ user = "admin", password = "infoblox" }})
        local ranges = c.range:get({{ network = "10.0.0.0/24" }})
        assert.eq(#ranges, 1)
        assert.eq(ranges[1].start_addr, "10.0.0.10")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_infoblox_grid_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wapi/v2.12/grid"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"_ref": "grid/b25l:demo-grid", "name": "demo-grid"}
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local infoblox = require("assay.infoblox")
        local c = infoblox.client("{}", {{ user = "admin", password = "infoblox" }})
        local grid = c.grid:status()
        assert.eq(#grid, 1)
        assert.eq(grid[1].name, "demo-grid")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_infoblox_create_record() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/wapi/v2.12/record:a"))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(serde_json::json!("record:a/ZG5z:new.example.com/default")),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local infoblox = require("assay.infoblox")
        local c = infoblox.client("{}", {{ user = "admin", password = "infoblox" }})
        local ref = c.records:create("record:a", {{ name = "new.example.com", ipv4addr = "10.0.0.9" }})
        assert.eq(ref, "record:a/ZG5z:new.example.com/default")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_infoblox_update_record() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/wapi/v2.12/record:a/ZG5z:host.example.com/default"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!("record:a/ZG5z:host.example.com/default")),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local infoblox = require("assay.infoblox")
        local c = infoblox.client("{}", {{ user = "admin", password = "infoblox" }})
        local ref = c.records:update("record:a/ZG5z:host.example.com/default", {{ ipv4addr = "10.0.0.6" }})
        assert.eq(ref, "record:a/ZG5z:host.example.com/default")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_infoblox_delete_record() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/wapi/v2.12/record:a/ZG5z:host.example.com/default"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!("record:a/ZG5z:host.example.com/default")),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local infoblox = require("assay.infoblox")
        local c = infoblox.client("{}", {{ user = "admin", password = "infoblox" }})
        local ref = c.records:delete("record:a/ZG5z:host.example.com/default")
        assert.eq(ref, "record:a/ZG5z:host.example.com/default")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_infoblox_error_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wapi/v2.12/record:a"))
        .respond_with(ResponseTemplate::new(500).set_body_string("grid error"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local infoblox = require("assay.infoblox")
        local c = infoblox.client("{}", {{ user = "admin", password = "infoblox" }})
        local ok, err = pcall(function() c.records:get("record:a", {{ name = "x.example.com" }}) end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "HTTP 500")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
