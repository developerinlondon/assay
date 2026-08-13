mod common;

use common::run_lua;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ITEMS_PATH: &str = "/api/v1/workspaces/acme/projects/p1/work-items/";

async fn mock_json(server: &MockServer, verb: &str, route: &str, body: serde_json::Value) {
    Mock::given(method(verb))
        .and(path(route.to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// One page of work items, optionally matched on the cursor the walker echoes back.
async fn mock_items_page(
    server: &MockServer,
    cursor: Option<&str>,
    results: serde_json::Value,
    next_cursor: Option<&str>,
    has_more: bool,
) {
    let body = serde_json::json!({
        "results": results,
        "next_cursor": next_cursor,
        "next_page_results": has_more,
    });
    let builder = Mock::given(method("GET")).and(path(ITEMS_PATH));
    let builder = match cursor {
        Some(cur) => builder.and(query_param("cursor", cur)),
        None => builder,
    };
    builder
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// The first page of a two-page walk: one item, more to come.
async fn mock_first_page(server: &MockServer) {
    mock_items_page(
        server,
        None,
        serde_json::json!([{"id": "i1", "name": "First"}]),
        Some("20:1:0"),
        true,
    )
    .await;
}

fn script(uri: &str, body: &str) -> String {
    format!(
        r#"
        local plane = require("assay.plane")
        local c = plane.client({{ api_key = "k", base_url = "{uri}", workspace = "acme" }})
        {body}
    "#
    )
}

#[tokio::test]
async fn test_require_plane() {
    let body = r#"
        local plane = require("assay.plane")
        assert.not_nil(plane.client)
        assert.not_nil(plane.all_items)
        assert.not_nil(plane.find_item_by_name)
        assert.not_nil(plane.ensure_item)
        assert.not_nil(plane.resolve_project)
    "#;
    run_lua(body).await.unwrap();
}

#[tokio::test]
async fn test_plane_client_sections() {
    let body = r#"
        local plane = require("assay.plane")
        local c = plane.client({ api_key = "k", workspace = "acme" })
        assert.not_nil(c.projects)
        assert.not_nil(c.items)
        assert.not_nil(c.states)
        assert.not_nil(c.labels)
        assert.not_nil(c.cycles)
        assert.not_nil(c.modules)
        assert.not_nil(c.members)
        assert.not_nil(c.comments)
        assert.not_nil(c.links)
        assert.not_nil(c.intake)
        assert.eq(c.workspace, "acme")
        assert.eq(c.authenticated, true)
    "#;
    run_lua(body).await.unwrap();
}

// Plane authenticates on X-API-Key. An Authorization header would be ignored.
#[tokio::test]
async fn test_plane_sends_api_key_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workspaces/acme/projects/"))
        .and(header("X-API-Key", "k"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"results": [{"id": "p1", "name": "Core"}]})),
        )
        .mount(&server)
        .await;

    let body = r#"
        assert.eq(#c.projects:list(), 1)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_plane_workspace_scoped_paths() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        "GET",
        "/api/v1/workspaces/acme/projects/p1/states/",
        serde_json::json!({"results": [{"id": "s1", "name": "Todo"}]}),
    )
    .await;
    mock_json(
        &server,
        "GET",
        "/api/v1/workspaces/acme/members/",
        serde_json::json!({"results": [{"id": "m1"}, {"id": "m2"}]}),
    )
    .await;

    let body = r#"
        assert.eq(c.states:list("p1")[1].name, "Todo")
        assert.eq(#c.members:list(), 2)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

// Collections answer with a cursor envelope; a bare array must also work.
#[tokio::test]
async fn test_plane_accepts_bare_array_collections() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        "GET",
        "/api/v1/workspaces/acme/projects/p1/labels/",
        serde_json::json!([{"id": "l1", "name": "bug"}]),
    )
    .await;

    let body = r#"
        assert.eq(c.labels:list("p1")[1].name, "bug")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_plane_items_page_exposes_cursor() {
    let server = MockServer::start().await;
    mock_first_page(&server).await;

    let body = r#"
        local page = c.items:page("p1")
        assert.eq(#page.items, 1)
        assert.eq(page.next_cursor, "20:1:0")
        assert.eq(page.has_more, true)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_plane_all_items_walks_cursor_then_stops() {
    let server = MockServer::start().await;
    // Cursor-matched mock is registered first so it wins over the catch-all.
    mock_items_page(
        &server,
        Some("20:1:0"),
        serde_json::json!([{"id": "i2", "name": "Second"}]),
        None,
        false,
    )
    .await;
    mock_first_page(&server).await;

    let body = r#"
        local plane = require("assay.plane")
        local all = plane.all_items(c, "p1")
        assert.eq(#all, 2)
        assert.eq(all[1].name, "First")
        assert.eq(all[2].name, "Second")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

// Work items live under /work-items/ but their comments and links hang off
// /issues/. Getting this wrong 404s, so it is worth locking down.
#[tokio::test]
async fn test_plane_comments_and_links_use_issues_path() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        "GET",
        "/api/v1/workspaces/acme/projects/p1/issues/i1/comments/",
        serde_json::json!({"results": [{"id": "c1"}]}),
    )
    .await;
    mock_json(
        &server,
        "GET",
        "/api/v1/workspaces/acme/projects/p1/issues/i1/links/",
        serde_json::json!({"results": [{"id": "k1", "url": "https://example.com"}]}),
    )
    .await;

    let body = r#"
        assert.eq(#c.comments:list("p1", "i1"), 1)
        assert.eq(c.links:list("p1", "i1")[1].url, "https://example.com")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_plane_cycle_add_items_posts_cycle_issues() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        "POST",
        "/api/v1/workspaces/acme/projects/p1/cycles/cy1/cycle-issues/",
        serde_json::json!({"ok": true}),
    )
    .await;

    let body = r#"
        assert.eq(c.cycles:add_items("p1", "cy1", { "i1", "i2" }), true)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_plane_missing_resource_returns_nil() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workspaces/acme/projects/p1/work-items/nope/"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let body = r#"
        assert.eq(c.items:get("p1", "nope"), nil)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_plane_ensure_item_is_idempotent() {
    let server = MockServer::start().await;
    mock_items_page(
        &server,
        None,
        serde_json::json!([{"id": "i1", "name": "Existing"}]),
        None,
        false,
    )
    .await;

    let body = r#"
        local plane = require("assay.plane")
        local got = plane.ensure_item(c, "p1", { name = "Existing" })
        assert.eq(got.id, "i1")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_plane_resolve_project_refuses_to_guess() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        "GET",
        "/api/v1/workspaces/acme/projects/",
        serde_json::json!({"results": [{"id": "p1", "name": "A"}, {"id": "p2", "name": "B"}]}),
    )
    .await;

    let body = r#"
        local plane = require("assay.plane")
        local ok = pcall(function() return plane.resolve_project(c) end)
        assert.eq(ok, false)
        assert.eq(plane.resolve_project(c, "B").id, "p2")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

// A missing workspace slug must fail loudly rather than build "/workspaces//".
#[tokio::test]
async fn test_plane_requires_workspace_slug() {
    let body = r#"
        local plane = require("assay.plane")
        local c = plane.client({ api_key = "k", workspace = "" })
        local ok = pcall(function() return c.projects:list() end)
        assert.eq(ok, false)
    "#;
    run_lua(body).await.unwrap();
}
