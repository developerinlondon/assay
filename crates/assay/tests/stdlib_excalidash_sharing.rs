mod common;

use common::excalidash::{
    CSRF_COOKIE, CSRF_COOKIE_VALUE, CSRF_TOKEN, JWT, count_calls, drawing, mount_csrf, mount_ok,
    ok, sent_bodies, sent_raw, session_script,
};
use common::run_lua;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ===== CSRF handshake =====

/// A session write must carry both halves of the CSRF pair — the token in its
/// header and the client cookie it is bound to. Either alone fails validation.
#[tokio::test]
async fn test_session_write_carries_csrf_token_and_cookie() {
    let server = MockServer::start().await;
    mount_csrf(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/drawings/d1/duplicate"))
        .and(header("x-csrf-token", CSRF_TOKEN))
        .and(header(
            "cookie",
            format!("{CSRF_COOKIE}={CSRF_COOKIE_VALUE}").as_str(),
        ))
        .and(header("authorization", format!("Bearer {JWT}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(drawing("d2", "Alpha (Copy)", 1)))
        .expect(1)
        .mount(&server)
        .await;

    let body = r#"
        assert.eq(c.drawings:duplicate("d1").id, "d2")
    "#;
    run_lua(&session_script(&server.uri(), body)).await.unwrap();
}

/// A session read needs no CSRF, so the handshake must not be paid for one —
/// and a write must pay it exactly once, not per call.
#[tokio::test]
async fn test_csrf_handshake_only_for_writes_and_only_once() {
    let server = MockServer::start().await;
    mount_csrf(&server).await;
    mount_ok(
        &server,
        "GET",
        "/drawings/d1/history",
        serde_json::json!({ "snapshots": [], "totalCount": 0 }),
    )
    .await;
    mount_ok(
        &server,
        "POST",
        "/drawings/d1/duplicate",
        drawing("d2", "Copy", 1),
    )
    .await;

    run_lua(&session_script(&server.uri(), r#"c.history:list("d1")"#))
        .await
        .unwrap();
    assert_eq!(
        count_calls(&server, "/csrf-token").await,
        0,
        "a read paid for a CSRF handshake"
    );

    let body = r#"
        c.drawings:duplicate("d1")
        c.drawings:duplicate("d1")
        c.drawings:duplicate("d1")
    "#;
    run_lua(&session_script(&server.uri(), body)).await.unwrap();
    assert_eq!(
        count_calls(&server, "/csrf-token").await,
        1,
        "handshake repeated per write"
    );
}

/// A handshake that answers a token but sets no cookie is unusable, and saying
/// so beats sending a half-pair the server will reject as invalid.
#[tokio::test]
async fn test_csrf_without_cookie_errors() {
    let server = MockServer::start().await;
    mount_ok(
        &server,
        "GET",
        "/csrf-token",
        serde_json::json!({ "token": CSRF_TOKEN, "header": "x-csrf-token" }),
    )
    .await;

    let body = r#"
        local ok, err = pcall(function() return c.drawings:duplicate("d1") end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "excalidash-csrf-client")
    "#;
    run_lua(&session_script(&server.uri(), body)).await.unwrap();
}

// ===== Version history =====

#[tokio::test]
async fn test_history_list_and_get() {
    let server = MockServer::start().await;
    mount_ok(
        &server,
        "GET",
        "/drawings/d1/history",
        serde_json::json!({
            "snapshots": [{ "id": "s1", "version": 2, "createdAt": "2026-08-13T15:55:38.094Z" }],
            "totalCount": 1,
        }),
    )
    .await;
    mount_ok(
        &server,
        "GET",
        "/drawings/d1/history/s1",
        serde_json::json!({
            "id": "s1", "drawingId": "d1", "version": 2,
            "elements": [{ "id": "el1" }], "appState": {}, "files": {},
        }),
    )
    .await;

    let body = r#"
        local h = c.history:list("d1", { limit = 10, offset = 0 })
        assert.eq(h.totalCount, 1)
        assert.eq(h.snapshots[1].id, "s1")
        assert.eq(h.snapshots[1].version, 2)

        local snap = c.history:get("d1", "s1")
        assert.eq(snap.drawingId, "d1")
        assert.eq(#snap.elements, 1)
    "#;
    run_lua(&session_script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_history_restore() {
    let server = MockServer::start().await;
    mount_csrf(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/drawings/d1/history/s1/restore"))
        .respond_with(ResponseTemplate::new(200).set_body_json(drawing("d1", "Alpha", 4)))
        .expect(1)
        .mount(&server)
        .await;

    run_lua(&session_script(
        &server.uri(),
        r#"assert.eq(c.history:restore("d1", "s1", 3).version, 4)"#,
    ))
    .await
    .unwrap();

    let sent = &sent_bodies(&server, "/drawings/d1/history/s1/restore").await[0];
    assert_eq!(
        sent["version"], 3,
        "restore did not carry the guard version"
    );
}

/// Servers from 0.6.0 on guard the restore with the drawing's current version
/// and answer 400 without one, so omitting it must leave the key off rather
/// than inventing a value that could clobber a newer scene.
#[tokio::test]
async fn test_history_restore_without_version_omits_the_key() {
    let server = MockServer::start().await;
    mount_csrf(&server).await;
    mount_ok(
        &server,
        "POST",
        "/drawings/d1/history/s1/restore",
        drawing("d1", "Alpha", 4),
    )
    .await;

    run_lua(&session_script(
        &server.uri(),
        r#"c.history:restore("d1", "s1")"#,
    ))
    .await
    .unwrap();

    let sent = &sent_bodies(&server, "/drawings/d1/history/s1/restore").await[0];
    assert!(
        sent.get("version").is_none(),
        "version should be absent, got {sent}"
    );
}

/// `undo_last_change` restores the newest snapshot, which is the state before
/// the last scene write — not the oldest one the window still holds.
#[tokio::test]
async fn test_undo_last_change_picks_the_newest_snapshot() {
    let server = MockServer::start().await;
    mount_csrf(&server).await;
    mount_ok(
        &server,
        "GET",
        "/drawings/d1/history",
        serde_json::json!({
            "snapshots": [
                { "id": "s2", "version": 3, "createdAt": "2026-08-13T16:00:00.000Z" },
                { "id": "s1", "version": 2, "createdAt": "2026-08-13T15:00:00.000Z" },
            ],
            "totalCount": 2,
        }),
    )
    .await;
    mount_ok(&server, "GET", "/drawings/d1", drawing("d1", "Alpha", 3)).await;
    Mock::given(method("POST"))
        .and(path("/api/drawings/d1/history/s2/restore"))
        .respond_with(ResponseTemplate::new(200).set_body_json(drawing("d1", "Alpha", 4)))
        .expect(1)
        .mount(&server)
        .await;

    let body = r#"
        local excalidash = require("assay.excalidash")
        assert.eq(excalidash.undo_last_change(c, "d1").version, 4)
    "#;
    run_lua(&session_script(&server.uri(), body)).await.unwrap();

    let sent = &sent_bodies(&server, "/drawings/d1/history/s2/restore").await[0];
    assert_eq!(
        sent["version"], 3,
        "undo did not guard on the current version"
    );
}

#[tokio::test]
async fn test_undo_last_change_without_history_is_nil() {
    let server = MockServer::start().await;
    mount_ok(
        &server,
        "GET",
        "/drawings/d1/history",
        serde_json::json!({ "snapshots": [], "totalCount": 0 }),
    )
    .await;

    let body = r#"
        local excalidash = require("assay.excalidash")
        assert.eq(excalidash.undo_last_change(c, "d1"), nil)
    "#;
    run_lua(&session_script(&server.uri(), body)).await.unwrap();
}

// ===== Sharing =====

#[tokio::test]
async fn test_sharing_get_returns_both_lists() {
    let server = MockServer::start().await;
    mount_ok(
        &server,
        "GET",
        "/drawings/d1/sharing",
        serde_json::json!({
            "permissions": [{ "id": "p1", "granteeUserId": "u1", "permission": "edit" }],
            "linkShares": [{ "id": "ls1", "permission": "view" }],
        }),
    )
    .await;

    let body = r#"
        local s = c.sharing:get("d1")
        assert.eq(#s.permissions, 1)
        assert.eq(s.permissions[1].permission, "edit")
        assert.eq(#s.linkShares, 1)
        assert.eq(s.linkShares[1].id, "ls1")
    "#;
    run_lua(&session_script(&server.uri(), body)).await.unwrap();
}

/// Grant answers `{permission: ...}`; the module returns the row itself. With
/// no permission named it defaults to view, not edit.
#[tokio::test]
async fn test_sharing_grant_unwraps_and_defaults_to_view() {
    let server = MockServer::start().await;
    mount_csrf(&server).await;
    mount_ok(
        &server,
        "POST",
        "/drawings/d1/permissions",
        serde_json::json!({
            "permission": { "id": "p1", "granteeUserId": "u1", "permission": "view" },
        }),
    )
    .await;

    let body = r#"
        local p = c.sharing:grant("d1", "u1")
        assert.eq(p.id, "p1")
        assert.eq(p.permission, "view")
    "#;
    run_lua(&session_script(&server.uri(), body)).await.unwrap();

    let sent = &sent_bodies(&server, "/drawings/d1/permissions").await[0];
    assert_eq!(sent["granteeUserId"], "u1");
    assert_eq!(sent["permission"], "view");
}

/// `expires_at = false` has to reach the server as a literal JSON null: an
/// absent key means "use the default TTL", and only an explicit null asks for
/// no expiry at all. assay's encoder has no null, so the module patches one in.
#[tokio::test]
async fn test_link_share_no_expiry_sends_json_null() {
    let server = MockServer::start().await;
    mount_csrf(&server).await;
    mount_ok(
        &server,
        "POST",
        "/drawings/d1/link-shares",
        serde_json::json!({ "share": { "id": "ls1", "permission": "view" } }),
    )
    .await;

    let body = r#"
        local s = c.sharing:create_link("d1", { permission = "view", expires_at = false })
        assert.eq(s.id, "ls1")
    "#;
    run_lua(&session_script(&server.uri(), body)).await.unwrap();

    let raw = sent_raw(&server, "/drawings/d1/link-shares").await;
    assert!(
        raw.contains("\"expiresAt\":null"),
        "expiresAt did not travel as JSON null: {raw}"
    );
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(parsed["expiresAt"].is_null());
    assert_eq!(parsed["permission"], "view");
    assert!(
        !raw.contains("excalidash_null"),
        "the null marker leaked into the body: {raw}"
    );
}

/// An explicit timestamp travels as a string, and omitting expiry entirely
/// leaves the key off so the server applies its own default TTL.
#[tokio::test]
async fn test_link_share_expiry_variants() {
    let server = MockServer::start().await;
    mount_csrf(&server).await;
    mount_ok(
        &server,
        "POST",
        "/drawings/d1/link-shares",
        serde_json::json!({ "share": { "id": "ls1" } }),
    )
    .await;

    let body = r#"
        c.sharing:create_link("d1", { permission = "edit", expires_at = "2026-09-01T00:00:00.000Z" })
        c.sharing:create_link("d1")
    "#;
    run_lua(&session_script(&server.uri(), body)).await.unwrap();

    let sent = sent_bodies(&server, "/drawings/d1/link-shares").await;
    assert_eq!(sent[0]["expiresAt"], "2026-09-01T00:00:00.000Z");
    assert_eq!(sent[0]["permission"], "edit");
    assert!(
        sent[1].get("expiresAt").is_none(),
        "expiresAt should be absent, got {}",
        sent[1]
    );
    assert_eq!(sent[1]["permission"], "view");
}

#[tokio::test]
async fn test_sharing_revoke_and_revoke_link() {
    let server = MockServer::start().await;
    mount_csrf(&server).await;
    mount_ok(&server, "DELETE", "/drawings/d1/permissions/p1", ok()).await;
    mount_ok(&server, "DELETE", "/drawings/d1/link-shares/ls1", ok()).await;

    let body = r#"
        assert.eq(c.sharing:revoke("d1", "p1"), true)
        assert.eq(c.sharing:revoke_link("d1", "ls1"), true)
    "#;
    run_lua(&session_script(&server.uri(), body)).await.unwrap();
    assert_eq!(count_calls(&server, "/drawings/d1/permissions/p1").await, 1);
    assert_eq!(
        count_calls(&server, "/drawings/d1/link-shares/ls1").await,
        1
    );
}

#[tokio::test]
async fn test_sharing_resolve_users() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/drawings/d1/share-resolve"))
        .and(query_param("q", "gra"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "users": [{ "id": "u1", "name": "Grantee", "email": "grantee@local" }],
        })))
        .mount(&server)
        .await;

    let body = r#"
        local users = c.sharing:resolve_users("d1", "gra")
        assert.eq(#users, 1)
        assert.eq(users[1].email, "grantee@local")
    "#;
    run_lua(&session_script(&server.uri(), body)).await.unwrap();
}

// ===== Collection sharing =====

#[tokio::test]
async fn test_collection_shares_lifecycle() {
    let server = MockServer::start().await;
    mount_csrf(&server).await;
    mount_ok(
        &server,
        "GET",
        "/collections/col-1/shares",
        serde_json::json!({ "shares": [{ "id": "cs1", "granteeUserId": "u1", "role": "view" }] }),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/collections/col-1/shares"))
        .and(body_json(
            serde_json::json!({ "identifier": "grantee@local", "role": "edit" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "share": { "id": "cs1", "granteeUserId": "u1", "role": "edit" } }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/api/collections/col-1/shares/u1"))
        .and(body_json(serde_json::json!({ "role": "view" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok()))
        .expect(1)
        .mount(&server)
        .await;
    mount_ok(&server, "DELETE", "/collections/col-1/shares/u1", ok()).await;

    let body = r#"
        local shares = c.collections:shares("col-1")
        assert.eq(#shares, 1)
        assert.eq(shares[1].role, "view")
        assert.eq(c.collections:share("col-1", "grantee@local", "edit").role, "edit")
        assert.eq(c.collections:set_share_role("col-1", "u1", "view"), true)
        assert.eq(c.collections:unshare("col-1", "u1"), true)
    "#;
    run_lua(&session_script(&server.uri(), body)).await.unwrap();
}

/// Sharing a collection with no role named defaults to view.
#[tokio::test]
async fn test_collection_share_defaults_to_view() {
    let server = MockServer::start().await;
    mount_csrf(&server).await;
    mount_ok(
        &server,
        "POST",
        "/collections/col-1/shares",
        serde_json::json!({ "share": { "id": "cs1", "role": "view" } }),
    )
    .await;

    run_lua(&session_script(
        &server.uri(),
        r#"c.collections:share("col-1", "a@b.com")"#,
    ))
    .await
    .unwrap();

    let sent = &sent_bodies(&server, "/collections/col-1/shares").await[0];
    assert_eq!(sent["role"], "view");
}
