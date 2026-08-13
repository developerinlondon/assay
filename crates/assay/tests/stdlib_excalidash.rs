mod common;

use common::excalidash::{
    JWT, KEY, count_verb, drawing, key_script, mount, mount_list, mount_ok, ok, page, sent_bodies,
    summary,
};
use common::run_lua;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ===== Module surface =====

#[tokio::test]
async fn test_require_excalidash() {
    let body = r#"
        local excalidash = require("assay.excalidash")
        assert.not_nil(excalidash.client)
        assert.not_nil(excalidash.all_drawings)
        assert.not_nil(excalidash.find_drawing_by_name)
        assert.not_nil(excalidash.ensure_drawing)
        assert.not_nil(excalidash.collections)
        assert.not_nil(excalidash.resolve_collection)
        assert.not_nil(excalidash.ensure_collection)
        assert.not_nil(excalidash.trash)
        assert.not_nil(excalidash.undo_last_change)
        assert.eq(excalidash.TRASH, "trash")
        assert.eq(excalidash.PERMISSIONS.view, "view")
        assert.eq(excalidash.PERMISSIONS.edit, "edit")
    "#;
    run_lua(body).await.unwrap();
}

#[tokio::test]
async fn test_client_fields_and_defaults() {
    let body = r#"
        local excalidash = require("assay.excalidash")
        local c = excalidash.client({ api_key = "k", base_url = "https://draw.example.com/" })
        assert.eq(c.base_url, "https://draw.example.com")
        assert.eq(c.api_path, "/api")
        assert.eq(c.has_api_key, true)
        assert.eq(c.has_session, false)

        local direct = excalidash.client({
          token = "t", base_url = "http://backend:8000", api_path = "",
        })
        assert.eq(direct.api_path, "")
        assert.eq(direct.has_api_key, false)
        assert.eq(direct.has_session, true)
    "#;
    run_lua(body).await.unwrap();
}

#[tokio::test]
async fn test_missing_base_url_errors() {
    let body = r#"
        local excalidash = require("assay.excalidash")
        local ok, err = pcall(function()
          return excalidash.client({ api_key = "k", base_url = "" })
        end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "no base url")
    "#;
    run_lua(body).await.unwrap();
}

#[tokio::test]
async fn test_missing_credential_errors() {
    let server = MockServer::start().await;
    let body = format!(
        r#"
        local excalidash = require("assay.excalidash")
        local c = excalidash.client({{ base_url = "{}" }})
        local ok, err = pcall(function() return c.drawings:list() end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "no credential")
    "#,
        server.uri()
    );
    run_lua(&body).await.unwrap();
    assert_eq!(count_verb(&server, wiremock::http::Method::GET).await, 0);
}

// ===== Drawings =====

#[tokio::test]
async fn test_drawings_list() {
    let server = MockServer::start().await;
    mount_list(&server, serde_json::json!([summary("d1", "Alpha")]), 1).await;

    let body = r#"
        local p = c.drawings:list()
        assert.eq(p.totalCount, 1)
        assert.eq(#p.drawings, 1)
        assert.eq(p.drawings[1].id, "d1")
        assert.eq(p.drawings[1].creatorName, "Probe")
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();
}

/// The API key travels as a bearer token, and nothing else authenticates.
#[tokio::test]
async fn test_api_key_sent_as_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/drawings"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(serde_json::json!([]), 0)))
        .expect(1)
        .mount(&server)
        .await;

    run_lua(&key_script(&server.uri(), "c.drawings:list()"))
        .await
        .unwrap();
}

/// List filters are renamed to the server's camelCase spelling, and booleans
/// and numbers travel as strings.
#[tokio::test]
async fn test_drawings_list_filters() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/drawings"))
        .and(query_param("search", "Alpha"))
        .and(query_param("collectionId", "col-1"))
        .and(query_param("includeData", "true"))
        .and(query_param("includePreview", "false"))
        .and(query_param("limit", "5"))
        .and(query_param("offset", "10"))
        .and(query_param("sortField", "name"))
        .and(query_param("sortDirection", "asc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(serde_json::json!([]), 0)))
        .expect(1)
        .mount(&server)
        .await;

    let body = r#"
        c.drawings:list({
          search = "Alpha", collection_id = "col-1",
          include_data = true, include_preview = false,
          limit = 5, offset = 10,
          sort_field = "name", sort_direction = "asc",
        })
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_drawings_get() {
    let server = MockServer::start().await;
    mount_ok(&server, "GET", "/drawings/d1", drawing("d1", "Alpha", 3)).await;

    let body = r##"
        local d = c.drawings:get("d1")
        assert.eq(d.id, "d1")
        assert.eq(d.version, 3)
        assert.eq(d.accessLevel, "owner")
        assert.eq(#d.elements, 1)
        assert.eq(d.appState.viewBackgroundColor, "#ffffff")
    "##;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();
}

/// A drawing that does not exist answers 404, which reads as nil rather than
/// raising — the caller asked whether it is there.
#[tokio::test]
async fn test_drawings_get_missing_is_nil() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/drawings/gone",
        404,
        serde_json::json!({ "error": "Drawing not found" }),
    )
    .await;

    run_lua(&key_script(
        &server.uri(),
        r#"assert.eq(c.drawings:get("gone"), nil)"#,
    ))
    .await
    .unwrap();
}

/// An id with a slash or a space must not escape its path segment.
#[tokio::test]
async fn test_drawing_id_is_url_encoded() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/drawings/a%2Fb%20c"))
        .respond_with(ResponseTemplate::new(200).set_body_json(drawing("a/b c", "X", 1)))
        .expect(1)
        .mount(&server)
        .await;

    run_lua(&key_script(&server.uri(), r#"c.drawings:get("a/b c")"#))
        .await
        .unwrap();
}

#[tokio::test]
async fn test_drawings_create() {
    let server = MockServer::start().await;
    mount_ok(&server, "POST", "/drawings", drawing("new-1", "Fresh", 1)).await;

    let body = r##"
        local d = c.drawings:create({
          name = "Fresh",
          collection_id = "col-1",
          elements = { { id = "el1", type = "rectangle" } },
          app_state = { viewBackgroundColor = "#fff" },
        })
        assert.eq(d.id, "new-1")
    "##;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();

    let sent = &sent_bodies(&server, "/drawings").await[0];
    assert_eq!(sent["name"], "Fresh");
    assert_eq!(sent["collectionId"], "col-1");
    assert_eq!(sent["elements"][0]["id"], "el1");
    assert_eq!(sent["appState"]["viewBackgroundColor"], "#fff");
}

/// An omitted or empty element list must encode as `[]`. Lua has one table
/// type, so an empty one would otherwise become `{}` and fail the server's
/// array schema.
#[tokio::test]
async fn test_create_encodes_empty_elements_as_array() {
    let server = MockServer::start().await;
    mount_ok(&server, "POST", "/drawings", drawing("new-1", "Blank", 1)).await;

    let body = r#"
        c.drawings:create({ name = "Blank" })
        c.drawings:create({ name = "Blank2", elements = {} })
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();

    for sent in sent_bodies(&server, "/drawings").await {
        assert!(
            sent["elements"].is_array(),
            "elements encoded as {} not an array",
            sent["elements"]
        );
        assert_eq!(sent["elements"].as_array().unwrap().len(), 0);
    }
}

/// Update sends only the keys the caller set, so a rename cannot blank a scene.
#[tokio::test]
async fn test_drawings_update_sends_only_given_keys() {
    let server = MockServer::start().await;
    mount_ok(&server, "PUT", "/drawings/d1", drawing("d1", "Renamed", 2)).await;

    let body = r#"
        local d = c.drawings:update("d1", { name = "Renamed" })
        assert.eq(d.name, "Renamed")
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();

    let sent = &sent_bodies(&server, "/drawings/d1").await[0];
    assert_eq!(sent["name"], "Renamed");
    assert_eq!(sent.as_object().unwrap().len(), 1, "sent {sent}");
}

/// Passing `version` makes the write conditional. A stale one answers 409, and
/// the error must carry the code so a caller can tell a conflict apart from a
/// validation failure and re-read before retrying.
#[tokio::test]
async fn test_version_conflict_surfaces_code() {
    let server = MockServer::start().await;
    mount(
        &server,
        "PUT",
        "/drawings/d1",
        409,
        serde_json::json!({
            "error": "Conflict", "code": "VERSION_CONFLICT",
            "message": "Drawing has changed since this editor state was loaded.",
            "currentVersion": 7,
        }),
    )
    .await;

    let body = r#"
        local ok, err = pcall(function()
          return c.drawings:update("d1", { elements = {}, version = 3 })
        end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "409")
        assert.contains(tostring(err), "VERSION_CONFLICT")
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();

    let sent = &sent_bodies(&server, "/drawings/d1").await[0];
    assert_eq!(sent["version"], 3);
}

#[tokio::test]
async fn test_drawings_delete() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/drawings/d1"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok()))
        .expect(1)
        .mount(&server)
        .await;

    run_lua(&key_script(
        &server.uri(),
        r#"assert.eq(c.drawings:delete("d1"), true)"#,
    ))
    .await
    .unwrap();
}

// ===== Collections =====

/// The collection list is a bare array, not an envelope, and always carries a
/// synthetic Trash entry.
#[tokio::test]
async fn test_collections_list() {
    let server = MockServer::start().await;
    mount_ok(
        &server,
        "GET",
        "/collections",
        serde_json::json!([
            { "id": "trash", "name": "Trash", "isOwner": true },
            { "id": "col-1", "name": "Diagrams", "isOwner": true, "isShared": false },
        ]),
    )
    .await;

    let body = r#"
        local cols = c.collections:list()
        assert.eq(#cols, 2)
        assert.eq(cols[1].id, "trash")
        assert.eq(cols[2].name, "Diagrams")
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_collections_create_rename_delete() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/collections"))
        .and(body_json(serde_json::json!({ "name": "Diagrams" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "id": "col-1", "name": "Diagrams", "isOwner": true }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/collections/col-1"))
        .and(body_json(serde_json::json!({ "name": "Sketches" })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "id": "col-1", "name": "Sketches" })),
        )
        .mount(&server)
        .await;
    mount_ok(&server, "DELETE", "/collections/col-1", ok()).await;

    let body = r#"
        local col = c.collections:create("Diagrams")
        assert.eq(col.id, "col-1")
        assert.eq(col.isOwner, true)
        assert.eq(c.collections:rename("col-1", "Sketches").name, "Sketches")
        assert.eq(c.collections:delete("col-1"), true)
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();
}

// ===== Credential routing =====

/// The server's scope gate refuses an API key everywhere but the four base
/// routes. The module says which credential is missing rather than letting the
/// call go out and come back as an opaque 401 or 403.
#[tokio::test]
async fn test_session_only_routes_refuse_an_api_key() {
    let server = MockServer::start().await;
    let body = r#"
        local cases = {
          { "version history", function() return c.history:list("d1") end },
          { "version history", function() return c.history:get("d1", "s1") end },
          { "version history", function() return c.history:restore("d1", "s1") end },
          { "drawing sharing", function() return c.sharing:get("d1") end },
          { "drawing sharing", function() return c.sharing:grant("d1", "u1", "edit") end },
          { "drawing sharing", function() return c.sharing:revoke("d1", "p1") end },
          { "drawing sharing", function() return c.sharing:create_link("d1") end },
          { "drawing sharing", function() return c.sharing:revoke_link("d1", "s1") end },
          { "drawing user lookup", function() return c.sharing:resolve_users("d1", "abc") end },
          { "collection sharing", function() return c.collections:shares("col-1") end },
          { "collection sharing", function() return c.collections:share("col-1", "a@b") end },
          { "collection sharing", function() return c.collections:unshare("col-1", "u1") end },
          { "duplicating a drawing", function() return c.drawings:duplicate("d1") end },
          { "the shared-with-me list", function() return c.drawings:shared() end },
        }
        for _, case in ipairs(cases) do
          local ok, err = pcall(case[2])
          assert.eq(ok, false)
          assert.contains(tostring(err), case[1])
          assert.contains(tostring(err), "not reachable with an API key")
        end
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();

    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "a session-only call reached the network with only an API key"
    );
}

/// With both credentials the API key answers the routes it can, because it
/// needs no CSRF round trip.
#[tokio::test]
async fn test_api_key_preferred_where_it_works() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/drawings"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(serde_json::json!([]), 0)))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/drawings/d1/sharing"))
        .and(header("authorization", format!("Bearer {JWT}").as_str()))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "permissions": [], "linkShares": [] })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let body = format!(
        r#"
        local excalidash = require("assay.excalidash")
        local c = excalidash.client({{ api_key = "{KEY}", token = "{JWT}", base_url = "{}" }})
        c.drawings:list()
        c.sharing:get("d1")
    "#,
        server.uri()
    );
    run_lua(&body).await.unwrap();
}

// ===== Helpers =====

/// Paging walks `offset` until a short page arrives.
#[tokio::test]
async fn test_all_drawings_walks_every_page() {
    let server = MockServer::start().await;
    let first: Vec<serde_json::Value> = (0..2).map(|i| summary(&format!("d{i}"), "X")).collect();
    Mock::given(method("GET"))
        .and(path("/api/drawings"))
        .and(query_param("offset", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(serde_json::json!(first), 3)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/drawings"))
        .and(query_param("offset", "2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(page(serde_json::json!([summary("d2", "X")]), 3)),
        )
        .mount(&server)
        .await;

    let body = r#"
        local excalidash = require("assay.excalidash")
        local all = excalidash.all_drawings(c, { limit = 2 })
        assert.eq(#all, 3)
        assert.eq(all[1].id, "d0")
        assert.eq(all[3].id, "d2")
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();
}

/// `search` is a substring filter server-side, so the exact-name lookup must
/// still compare — a query for "Alpha" also returns "Alpha Two".
#[tokio::test]
async fn test_find_drawing_by_name_is_exact() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/drawings"))
        .and(query_param("search", "Alpha"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(
            serde_json::json!([summary("d2", "Alpha Two"), summary("d1", "Alpha")]),
            2,
        )))
        .mount(&server)
        .await;

    let body = r#"
        local excalidash = require("assay.excalidash")
        local found = excalidash.find_drawing_by_name(c, "Alpha")
        assert.eq(found.id, "d1")
        assert.eq(found.name, "Alpha")
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();
}

/// An existing name is returned as-is; nothing is created.
#[tokio::test]
async fn test_ensure_drawing_returns_existing() {
    let server = MockServer::start().await;
    mount_list(&server, serde_json::json!([summary("d1", "Alpha")]), 1).await;

    let body = r#"
        local excalidash = require("assay.excalidash")
        assert.eq(excalidash.ensure_drawing(c, { name = "Alpha" }).id, "d1")
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();

    assert_eq!(
        count_verb(&server, wiremock::http::Method::POST).await,
        0,
        "ensure_drawing created a duplicate"
    );
}

#[tokio::test]
async fn test_ensure_drawing_creates_when_absent() {
    let server = MockServer::start().await;
    mount_list(&server, serde_json::json!([]), 0).await;
    Mock::given(method("POST"))
        .and(path("/api/drawings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(drawing("new-1", "Alpha", 1)))
        .expect(1)
        .mount(&server)
        .await;

    let body = r#"
        local excalidash = require("assay.excalidash")
        assert.eq(excalidash.ensure_drawing(c, { name = "Alpha" }).id, "new-1")
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_ensure_drawing_requires_a_name() {
    let body = r#"
        local excalidash = require("assay.excalidash")
        local c = excalidash.client({ api_key = "k", base_url = "https://d.example.com" })
        local ok, err = pcall(function() return excalidash.ensure_drawing(c, {}) end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "requires spec.name")
    "#;
    run_lua(body).await.unwrap();
}

/// Trash exists on every account and is never a name to resolve against, so
/// the helper leaves it out — as it does collections owned by someone else.
#[tokio::test]
async fn test_collections_helper_excludes_trash_and_foreign() {
    let server = MockServer::start().await;
    mount_ok(
        &server,
        "GET",
        "/collections",
        serde_json::json!([
            { "id": "trash", "name": "Trash", "isOwner": true },
            { "id": "col-1", "name": "Diagrams", "isOwner": true },
            { "id": "col-2", "name": "Theirs", "isOwner": false, "sharedRole": "view" },
        ]),
    )
    .await;

    let body = r#"
        local excalidash = require("assay.excalidash")
        local cols = excalidash.collections(c)
        assert.eq(#cols, 1)
        assert.eq(cols[1].id, "col-1")
        assert.eq(excalidash.resolve_collection(c).id, "col-1")
        assert.eq(excalidash.resolve_collection(c, "Diagrams").id, "col-1")
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_resolve_collection_refuses_to_guess() {
    let server = MockServer::start().await;
    mount_ok(
        &server,
        "GET",
        "/collections",
        serde_json::json!([
            { "id": "col-1", "name": "One", "isOwner": true },
            { "id": "col-2", "name": "Two", "isOwner": true },
        ]),
    )
    .await;

    let body = r#"
        local excalidash = require("assay.excalidash")
        local ok, err = pcall(function() return excalidash.resolve_collection(c) end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "2 collections")

        local ok2, err2 = pcall(function() return excalidash.resolve_collection(c, "Three") end)
        assert.eq(ok2, false)
        assert.contains(tostring(err2), "no collection named Three")
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_ensure_collection() {
    let server = MockServer::start().await;
    mount_ok(
        &server,
        "GET",
        "/collections",
        serde_json::json!([{ "id": "col-1", "name": "Diagrams", "isOwner": true }]),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/collections"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "id": "col-9", "name": "New" })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let body = r#"
        local excalidash = require("assay.excalidash")
        assert.eq(excalidash.ensure_collection(c, "Diagrams").id, "col-1")
        assert.eq(excalidash.ensure_collection(c, "New").id, "col-9")
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();
}

/// Trashing is an ordinary collection move, and the public id for trash is the
/// bare word — the server maps it to `trash:<userId>` on both sides.
#[tokio::test]
async fn test_trash_moves_rather_than_deletes() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/drawings/d1"))
        .and(body_json(serde_json::json!({ "collectionId": "trash" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(drawing("d1", "Alpha", 3)))
        .expect(1)
        .mount(&server)
        .await;

    let body = r#"
        local excalidash = require("assay.excalidash")
        assert.eq(excalidash.trash(c, "d1").id, "d1")
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();

    assert_eq!(
        count_verb(&server, wiremock::http::Method::DELETE).await,
        0,
        "trash deleted the drawing"
    );
}

// ===== Paths and errors =====

/// A dashboard origin proxies the backend under /api; pointing at the backend
/// directly needs that prefix gone.
#[tokio::test]
async fn test_empty_api_path_targets_the_backend_directly() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/drawings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(serde_json::json!([]), 0)))
        .expect(1)
        .mount(&server)
        .await;

    let body = format!(
        r#"
        local excalidash = require("assay.excalidash")
        local c = excalidash.client({{ api_key = "{KEY}", base_url = "{}", api_path = "" }})
        c.drawings:list()
    "#,
        server.uri()
    );
    run_lua(&body).await.unwrap();
}

/// A server-side failure names the verb, the route, the status and the server's
/// own message, so a script's log says what went wrong without a second call.
#[tokio::test]
async fn test_error_carries_route_status_and_message() {
    let server = MockServer::start().await;
    mount(
        &server,
        "POST",
        "/collections",
        400,
        serde_json::json!({
            "error": "Validation error",
            "message": "Collection name must be between 1 and 100 characters",
        }),
    )
    .await;

    let body = r#"
        local ok, err = pcall(function() return c.collections:create("") end)
        assert.eq(ok, false)
        local msg = tostring(err)
        assert.contains(msg, "POST")
        assert.contains(msg, "/collections")
        assert.contains(msg, "400")
        assert.contains(msg, "Validation error")
        assert.contains(msg, "between 1 and 100")
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();
}

/// A dashboard origin answers any unknown path with the SPA's HTML and a 200.
/// Pointed at the wrong prefix, a read would otherwise decode to nil and report
/// an empty dashboard — a misconfiguration disguised as a fact.
#[tokio::test]
async fn test_html_body_is_refused_rather_than_read_as_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/drawings"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("<!doctype html><html><body>app</body></html>")
                .insert_header("content-type", "text/html"),
        )
        .mount(&server)
        .await;

    let body = r#"
        local ok, err = pcall(function() return c.drawings:list() end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "non-JSON body")
        assert.contains(tostring(err), "api_path")
    "#;
    run_lua(&key_script(&server.uri(), body)).await.unwrap();
}
