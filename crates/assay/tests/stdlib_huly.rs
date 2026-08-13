mod common;

use common::run_lua;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const WS: &str = "b07d7630-3393-44e8-803d-4366baaeb80b";

fn find_path() -> String {
    format!("/_transactor/api/v1/find-all/{WS}")
}

fn tx_path() -> String {
    format!("/_transactor/api/v1/tx/{WS}")
}

/// The transactor's collection envelope. Everything read comes wrapped in it.
fn total_array(value: serde_json::Value, total: i64) -> serde_json::Value {
    serde_json::json!({
        "dataType": "TotalArray",
        "total": total,
        "lookupMap": null,
        "value": value,
    })
}

async fn mock_find(server: &MockServer, class: &str, value: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(find_path()))
        .and(query_param("class", class))
        .respond_with(ResponseTemplate::new(200).set_body_json(total_array(value, -1)))
        .mount(server)
        .await;
}

/// Transactions answer a bare `[]` on success.
async fn mock_tx_ok(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path(tx_path()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(server)
        .await;
}

/// Answer a transaction POST only when its `_class` matches, so the sequence
/// bump and the document create can answer differently on the same route.
async fn mock_tx_class(server: &MockServer, tx_class: &'static str, body: serde_json::Value) {
    Mock::given(method("POST"))
        .and(path(tx_path()))
        .and(TxClass(tx_class))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Match on a key being present in the JSON `query` parameter, so two reads in
/// one flow can be told apart without depending on Lua's table order.
struct QueryHas(&'static str);

impl wiremock::Match for QueryHas {
    fn matches(&self, req: &wiremock::Request) -> bool {
        req.url
            .query_pairs()
            .find(|(k, _)| k == "query")
            .and_then(|(_, v)| serde_json::from_str::<serde_json::Value>(&v).ok())
            .and_then(|v| v.get(self.0).cloned())
            .is_some()
    }
}

struct TxClass(&'static str);

impl wiremock::Match for TxClass {
    fn matches(&self, req: &wiremock::Request) -> bool {
        serde_json::from_slice::<serde_json::Value>(&req.body)
            .ok()
            .and_then(|v| v["_class"].as_str().map(|s| s == self.0))
            .unwrap_or(false)
    }
}

/// Every transaction body the server received, in order.
async fn sent_txs(server: &MockServer) -> Vec<serde_json::Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().contains("/tx/"))
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

/// Lua tables have no key order, so a JSON parameter built from one must be
/// compared structurally rather than as a string.
async fn sent_json_param(server: &MockServer, name: &str) -> serde_json::Value {
    let requests = server.received_requests().await.unwrap();
    let req = requests
        .iter()
        .find(|r| r.url.query_pairs().any(|(k, _)| k == name))
        .unwrap_or_else(|| panic!("no request carried a {name} parameter"));
    let raw = req
        .url
        .query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}

fn tx_of_class<'a>(txs: &'a [serde_json::Value], tx_class: &str) -> &'a serde_json::Value {
    txs.iter()
        .find(|t| t["_class"] == tx_class)
        .unwrap_or_else(|| panic!("no {tx_class} transaction was sent"))
}

fn script(uri: &str, body: &str) -> String {
    format!(
        r#"
        local huly = require("assay.huly")
        local c = huly.client({{
          token = "t", workspace = "{WS}", account = "actor-1", base_url = "{uri}",
        }})
        {body}
    "#
    )
}

#[tokio::test]
async fn test_require_huly() {
    let body = r#"
        local huly = require("assay.huly")
        assert.not_nil(huly.client)
        assert.not_nil(huly.new_id)
        assert.not_nil(huly.projects)
        assert.not_nil(huly.resolve_project)
        assert.not_nil(huly.issues)
        assert.not_nil(huly.create_issue)
        assert.not_nil(huly.ensure_issue)
        assert.not_nil(huly.find_issue_by_title)
        assert.not_nil(huly.set_issue_status)
        assert.not_nil(huly.statuses)
    "#;
    run_lua(body).await.unwrap();
}

#[tokio::test]
async fn test_huly_client_fields() {
    let body = r#"
        local huly = require("assay.huly")
        local c = huly.client({ token = "t", workspace = "ws-1", base_url = "https://huly.example.com" })
        assert.eq(c.workspace, "ws-1")
        assert.eq(c.authenticated, true)
        assert.eq(c.endpoint, "https://huly.example.com/_transactor")

        local anon = huly.client({ workspace = "ws-1", base_url = "https://huly.example.com/" })
        assert.eq(anon.authenticated, false)
        assert.eq(anon.endpoint, "https://huly.example.com/_transactor")
    "#;
    run_lua(body).await.unwrap();
}

/// A base_url that already names the transactor must not gain a second segment.
#[tokio::test]
async fn test_huly_endpoint_not_doubled() {
    let body = r#"
        local huly = require("assay.huly")
        local c = huly.client({ workspace = "ws", base_url = "https://h.example.com/_transactor" })
        assert.eq(c.endpoint, "https://h.example.com/_transactor")
    "#;
    run_lua(body).await.unwrap();
}

/// Ids must be 24 hex characters; the transactor's `isId` rejects anything else.
#[tokio::test]
async fn test_huly_new_id_shape() {
    let body = r#"
        local huly = require("assay.huly")
        local a = huly.new_id()
        local b = huly.new_id()
        assert.eq(#a, 24)
        assert.not_nil(a:match("^[0-9a-f][0-9a-f]*$"))
        assert.eq(#a:gsub("[0-9a-f]", ""), 0)
        assert.ne(a, b)
    "#;
    run_lua(body).await.unwrap();
}

#[tokio::test]
async fn test_huly_missing_workspace_errors() {
    let body = r#"
        local huly = require("assay.huly")
        local c = huly.client({ token = "t", workspace = "", base_url = "https://h.example.com" })
        local ok, err = pcall(function() return c:account() end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "no workspace uuid")
    "#;
    run_lua(body).await.unwrap();
}

/// Bearer auth, and identity encoding — assay has neither a snappy nor a gzip
/// decoder, so a compressed response would arrive as unparseable bytes.
#[tokio::test]
async fn test_huly_sends_bearer_and_identity_encoding() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/_transactor/api/v1/account/{WS}")))
        .and(header("Authorization", "Bearer t"))
        .and(header("Accept-Encoding", "identity"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "uuid": "u1", "primarySocialId": "sid-1", "socialIds": ["sid-1"],
        })))
        .mount(&server)
        .await;

    let body = r#"
        local acc = c:account()
        assert.eq(acc.uuid, "u1")
        assert.eq(acc.primarySocialId, "sid-1")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_huly_find_all_unwraps_envelope() {
    let server = MockServer::start().await;
    mock_find(
        &server,
        "tracker:class:Issue",
        serde_json::json!([
            {"_id": "i1", "title": "One"},
            {"_id": "i2", "title": "Two"},
        ]),
    )
    .await;

    let body = r#"
        local docs = c:find_all("tracker:class:Issue")
        assert.eq(#docs, 2)
        assert.eq(docs[1].title, "One")
        assert.eq(docs[2]._id, "i2")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

/// The transactor omits `_class` on class-scoped reads, and strips attributes
/// the query already pins to a scalar. Both are re-injected client-side.
#[tokio::test]
async fn test_huly_restores_class_and_scalar_query_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(find_path()))
        .and(query_param("class", "tracker:class:Issue"))
        .respond_with(ResponseTemplate::new(200).set_body_json(total_array(
            serde_json::json!([{"_id": "i1", "title": "One"}]),
            1,
        )))
        .mount(&server)
        .await;

    let body = r#"
        local docs = c:find_all("tracker:class:Issue",
          { identifier = "TSK-1", number = 1, done = false })
        assert.eq(#docs, 1)
        assert.eq(docs[1]._class, "tracker:class:Issue")
        assert.eq(docs[1].identifier, "TSK-1")
        assert.eq(docs[1].number, 1)
        assert.eq(docs[1].done, false)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();

    assert_eq!(
        sent_json_param(&server, "query").await,
        serde_json::json!({"identifier": "TSK-1", "number": 1, "done": false}),
    );
}

/// Re-injection must not clobber a value the server did send.
#[tokio::test]
async fn test_huly_restore_does_not_overwrite_server_values() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(find_path()))
        .and(query_param("class", "tracker:class:Issue"))
        .respond_with(ResponseTemplate::new(200).set_body_json(total_array(
            serde_json::json!([{"_id": "i1", "_class": "tracker:mixin:IssueTypeData", "priority": 3}]),
            1,
        )))
        .mount(&server)
        .await;

    let body = r#"
        local docs = c:find_all("tracker:class:Issue", { priority = 3 })
        assert.eq(docs[1]._class, "tracker:mixin:IssueTypeData")
        assert.eq(docs[1].priority, 3)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

/// Table-valued query terms are not scalars and must not be copied onto results.
#[tokio::test]
async fn test_huly_restore_skips_operator_query_terms() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(find_path()))
        .and(query_param("class", "tracker:class:Issue"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(total_array(serde_json::json!([{"_id": "i1"}]), 1)),
        )
        .mount(&server)
        .await;

    let body = r#"
        local docs = c:find_all("tracker:class:Issue", { priority = { ["$in"] = { 1, 2 } } })
        assert.eq(docs[1].priority, nil)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_huly_find_one_limits_to_one() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(find_path()))
        .and(query_param("class", "tracker:class:Issue"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(total_array(serde_json::json!([{"_id": "i1"}]), 1)),
        )
        .mount(&server)
        .await;

    let body = r#"
        assert.eq(c:find_one("tracker:class:Issue")._id, "i1")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();

    assert_eq!(
        sent_json_param(&server, "options").await,
        serde_json::json!({"limit": 1}),
    );
}

/// `total` stays -1 unless the request asks for it, so `count` always asks.
#[tokio::test]
async fn test_huly_count_requests_total() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(find_path()))
        .and(query_param("class", "tracker:class:Issue"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(total_array(serde_json::json!([{"_id": "i1"}]), 42)),
        )
        .mount(&server)
        .await;

    let body = r#"
        assert.eq(c:count("tracker:class:Issue"), 42)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();

    assert_eq!(
        sent_json_param(&server, "options").await,
        serde_json::json!({"limit": 1, "total": true}),
    );
}

#[tokio::test]
async fn test_huly_find_all_requires_a_class() {
    let body = r#"
        local ok, err = pcall(function() return c:find_all("") end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "requires a class id")
    "#;
    run_lua(&script("https://h.example.com", body))
        .await
        .unwrap();
}

/// An unknown class answers 404 with a JSON body naming the failure; the module
/// must raise it rather than silently returning nothing.
#[tokio::test]
async fn test_huly_surfaces_api_error_detail() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(find_path()))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "message": "Failed to execute operation",
            "error": "Invalid class name is passed. Failed to findAll.",
        })))
        .mount(&server)
        .await;

    let body = r#"
        local ok, err = pcall(function() return c:find_all("bogus:class:Nope") end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "404")
        assert.contains(tostring(err), "Invalid class name is passed")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_huly_create_doc_tx_shape() {
    let server = MockServer::start().await;
    mock_tx_ok(&server).await;

    let body = r#"
        local id = c:create_doc("tracker:class:Component", "tracker:project:DefaultProject",
          { label = "Infra" }, "fixed-object-id")
        assert.eq(id, "fixed-object-id")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();

    let tx = sent_txs(&server).await.remove(0);
    assert_eq!(tx["_class"], "core:class:TxCreateDoc");
    assert_eq!(tx["space"], "core:space:Tx");
    assert_eq!(tx["objectId"], "fixed-object-id");
    assert_eq!(tx["objectClass"], "tracker:class:Component");
    assert_eq!(tx["objectSpace"], "tracker:project:DefaultProject");
    assert_eq!(tx["attributes"]["label"], "Infra");
    assert_eq!(tx["modifiedBy"], "actor-1");
    assert_eq!(tx["createdBy"], "actor-1");
    assert_eq!(tx["_id"].as_str().unwrap().len(), 24);
    assert!(tx["modifiedOn"].as_i64().unwrap() > 1_700_000_000_000);
}

#[tokio::test]
async fn test_huly_create_doc_generates_an_id() {
    let server = MockServer::start().await;
    mock_tx_ok(&server).await;

    let body = r#"
        local id = c:create_doc("tracker:class:Component", "space-1", { label = "X" })
        assert.eq(#id, 24)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();

    let tx = sent_txs(&server).await.remove(0);
    assert_eq!(tx["objectId"].as_str().unwrap().len(), 24);
    assert_ne!(tx["objectId"], tx["_id"]);
}

#[tokio::test]
async fn test_huly_update_doc_tx_shape() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(tx_path()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": {"_id": "p1", "sequence": 7},
        })))
        .mount(&server)
        .await;

    let body = r#"
        local res = c:update_doc("tracker:class:Project", "core:space:Space", "p1",
          { ["$inc"] = { sequence = 1 } }, true)
        assert.eq(res.object.sequence, 7)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();

    let tx = sent_txs(&server).await.remove(0);
    assert_eq!(tx["_class"], "core:class:TxUpdateDoc");
    assert_eq!(tx["objectId"], "p1");
    assert_eq!(tx["operations"]["$inc"]["sequence"], 1);
    assert_eq!(tx["retrieve"], true);
    assert!(tx.get("createdBy").is_none());
}

#[tokio::test]
async fn test_huly_remove_doc_tx_shape() {
    let server = MockServer::start().await;
    mock_tx_ok(&server).await;

    let body = r#"
        assert.eq(c:remove_doc("tracker:class:Issue", "proj-1", "i1"), true)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();

    let tx = sent_txs(&server).await.remove(0);
    assert_eq!(tx["_class"], "core:class:TxRemoveDoc");
    assert_eq!(tx["objectId"], "i1");
    assert_eq!(tx["objectSpace"], "proj-1");
}

/// Without an explicit account the client resolves the token's primary social
/// id — the transactor stamps `modifiedBy` with a social id, not a person uuid.
#[tokio::test]
async fn test_huly_actor_falls_back_to_primary_social_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/_transactor/api/v1/account/{WS}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "uuid": "person-uuid", "primarySocialId": "sid-9", "socialIds": ["sid-9", "sid-8"],
        })))
        .mount(&server)
        .await;
    mock_tx_ok(&server).await;

    let body = format!(
        r#"
        local huly = require("assay.huly")
        local c = huly.client({{ token = "t", workspace = "{WS}", base_url = "{uri}" }})
        assert.eq(c:actor(), "sid-9")
        c:create_doc("tracker:class:Component", "s", {{ label = "X" }})
    "#,
        uri = server.uri()
    );
    run_lua(&body).await.unwrap();

    assert_eq!(sent_txs(&server).await[0]["modifiedBy"], "sid-9");
}

#[tokio::test]
async fn test_huly_search_fulltext() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/_transactor/api/v1/search-fulltext/{WS}")))
        .and(query_param("query", "probe"))
        .and(query_param("limit", "5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "docs": [{"id": "i1", "title": "assay probe issue", "shortTitle": "TSK-1"}],
            "total": 1,
        })))
        .mount(&server)
        .await;

    let body = r#"
        local res = c:search("probe", { limit = 5 })
        assert.eq(res.total, 1)
        assert.eq(res.docs[1].shortTitle, "TSK-1")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_huly_ensure_person_posts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/_transactor/api/v1/ensure-person/{WS}")))
        .and(body_json(serde_json::json!({
            "socialType": "email",
            "socialValue": "probe@example.com",
            "firstName": "Probe",
            "lastName": "User",
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "uuid": "p-uuid", "socialId": "sid-3", "localPerson": "lp-1",
        })))
        .mount(&server)
        .await;

    let body = r#"
        local p = c:ensure_person("email", "probe@example.com", "Probe", "User")
        assert.eq(p.socialId, "sid-3")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

// ===== Tracker helpers =====

#[tokio::test]
async fn test_huly_resolve_project_by_identifier_and_name() {
    let server = MockServer::start().await;
    mock_find(
        &server,
        "tracker:class:Project",
        serde_json::json!([
            {"_id": "p1", "identifier": "TSK", "name": "Default"},
            {"_id": "p2", "identifier": "OPS", "name": "Operations"},
        ]),
    )
    .await;

    let body = r#"
        local huly = require("assay.huly")
        assert.eq(huly.resolve_project(c, "OPS")._id, "p2")
        assert.eq(huly.resolve_project(c, "Default")._id, "p1")

        local ok, err = pcall(function() return huly.resolve_project(c, "nope") end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "no project with identifier or name nope")

        local ok2, err2 = pcall(function() return huly.resolve_project(c) end)
        assert.eq(ok2, false)
        assert.contains(tostring(err2), "sees 2 projects")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_huly_resolve_project_single() {
    let server = MockServer::start().await;
    mock_find(
        &server,
        "tracker:class:Project",
        serde_json::json!([{"_id": "p1", "identifier": "TSK", "name": "Default"}]),
    )
    .await;

    let body = r#"
        local huly = require("assay.huly")
        assert.eq(huly.resolve_project(c).identifier, "TSK")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_huly_resolve_project_none_visible() {
    let server = MockServer::start().await;
    mock_find(&server, "tracker:class:Project", serde_json::json!([])).await;

    let body = r#"
        local huly = require("assay.huly")
        local ok, err = pcall(function() return huly.resolve_project(c) end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "can see no tracker projects")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_huly_issues_scoped_to_project_space() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(find_path()))
        .and(query_param("class", "tracker:class:Issue"))
        .and(query_param("query", r#"{"space":"p1"}"#))
        .respond_with(ResponseTemplate::new(200).set_body_json(total_array(
            serde_json::json!([{"_id": "i1", "title": "One"}]),
            1,
        )))
        .mount(&server)
        .await;

    let body = r#"
        local huly = require("assay.huly")
        local issues = huly.issues(c, { _id = "p1" })
        assert.eq(#issues, 1)
        assert.eq(issues[1].space, "p1")
        assert.eq(#huly.issues(c, "p1"), 1)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

/// Issue numbering is the project's, not the issue's: create bumps the
/// project's `sequence` and derives both `number` and `PREFIX-N` from it.
#[tokio::test]
async fn test_huly_create_issue_numbers_from_project_sequence() {
    let server = MockServer::start().await;
    mock_tx_class(
        &server,
        "core:class:TxUpdateDoc",
        serde_json::json!({"object": {"_id": "p1", "identifier": "TSK", "sequence": 5}}),
    )
    .await;
    mock_tx_class(&server, "core:class:TxCreateDoc", serde_json::json!([])).await;
    Mock::given(method("GET"))
        .and(path(find_path()))
        .and(query_param("class", "tracker:class:Issue"))
        .respond_with(ResponseTemplate::new(200).set_body_json(total_array(
            serde_json::json!([{"_id": "i1", "identifier": "TSK-5", "number": 5}]),
            1,
        )))
        .mount(&server)
        .await;

    let body = r#"
        local huly = require("assay.huly")
        local issue = huly.create_issue(c,
          { _id = "p1", identifier = "TSK", space = "core:space:Space" },
          { title = "Ship it", priority = 2 })
        assert.eq(issue.identifier, "TSK-5")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();

    let txs = sent_txs(&server).await;

    let inc = tx_of_class(&txs, "core:class:TxUpdateDoc");
    assert_eq!(inc["operations"]["$inc"]["sequence"], 1);
    assert_eq!(inc["retrieve"], true);

    let create = tx_of_class(&txs, "core:class:TxCreateDoc");
    let attrs = &create["attributes"];
    assert_eq!(create["objectClass"], "tracker:class:Issue");
    assert_eq!(create["objectSpace"], "p1");
    assert_eq!(attrs["number"], 5);
    assert_eq!(attrs["identifier"], "TSK-5");
    assert_eq!(attrs["priority"], 2);
    assert_eq!(attrs["title"], "Ship it");
    assert_eq!(attrs["status"], "tracker:status:Backlog");
    assert_eq!(attrs["kind"], "tracker:taskTypes:Issue");
    assert_eq!(attrs["attachedTo"], "tracker:ids:NoParent");
    assert_eq!(attrs["attachedToClass"], "tracker:class:Issue");
    assert_eq!(attrs["collection"], "subIssues");
    assert_eq!(attrs["rank"], "0|hzzzzz:");
    // Empty collections must serialise as arrays; `{}` would be an object.
    assert!(attrs["childInfo"].is_array());
    assert!(attrs["parents"].is_array());
}

/// The project's own default status wins over the module's fallback.
#[tokio::test]
async fn test_huly_create_issue_uses_project_default_status() {
    let server = MockServer::start().await;
    mock_tx_class(
        &server,
        "core:class:TxUpdateDoc",
        serde_json::json!({"object": {"sequence": 1}}),
    )
    .await;
    mock_tx_class(&server, "core:class:TxCreateDoc", serde_json::json!([])).await;
    mock_find(
        &server,
        "tracker:class:Issue",
        serde_json::json!([{"_id": "i1"}]),
    )
    .await;

    let body = r#"
        local huly = require("assay.huly")
        huly.create_issue(c,
          { _id = "p1", identifier = "OPS", defaultIssueStatus = "tracker:status:Todo" },
          { title = "T" })
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();

    let txs = sent_txs(&server).await;
    let create = tx_of_class(&txs, "core:class:TxCreateDoc");
    assert_eq!(create["attributes"]["status"], "tracker:status:Todo");
}

#[tokio::test]
async fn test_huly_create_issue_requires_a_title() {
    let body = r#"
        local huly = require("assay.huly")
        local ok, err = pcall(function()
          return huly.create_issue(c, { _id = "p1", identifier = "T" }, {})
        end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "requires spec.title")
    "#;
    run_lua(&script("https://h.example.com", body))
        .await
        .unwrap();
}

/// A project that does not answer with a sequence must fail loudly rather than
/// writing an unnumbered issue the UI cannot show.
#[tokio::test]
async fn test_huly_create_issue_fails_without_a_sequence() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(tx_path()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let body = r#"
        local huly = require("assay.huly")
        local ok, err = pcall(function()
          return huly.create_issue(c, { _id = "p1", identifier = "T" }, { title = "X" })
        end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "did not return a sequence")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

/// ensure_issue is idempotent: an existing title means no transaction at all.
#[tokio::test]
async fn test_huly_ensure_issue_returns_existing() {
    let server = MockServer::start().await;
    mock_find(
        &server,
        "tracker:class:Issue",
        serde_json::json!([{"_id": "i1", "title": "Ship it"}]),
    )
    .await;

    let body = r#"
        local huly = require("assay.huly")
        local issue = huly.ensure_issue(c, { _id = "p1", identifier = "TSK" }, { title = "Ship it" })
        assert.eq(issue._id, "i1")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();

    assert!(
        sent_txs(&server).await.is_empty(),
        "ensure_issue must not write when the title exists"
    );
}

#[tokio::test]
async fn test_huly_ensure_issue_creates_when_absent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(find_path()))
        .and(query_param("class", "tracker:class:Issue"))
        .and(QueryHas("title"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(total_array(serde_json::json!([]), 0)),
        )
        .mount(&server)
        .await;
    mock_tx_class(
        &server,
        "core:class:TxUpdateDoc",
        serde_json::json!({"object": {"sequence": 3}}),
    )
    .await;
    mock_tx_class(&server, "core:class:TxCreateDoc", serde_json::json!([])).await;
    Mock::given(method("GET"))
        .and(path(find_path()))
        .and(query_param("class", "tracker:class:Issue"))
        .and(QueryHas("_id"))
        .respond_with(ResponseTemplate::new(200).set_body_json(total_array(
            serde_json::json!([{"_id": "i9", "identifier": "TSK-3"}]),
            1,
        )))
        .mount(&server)
        .await;

    let body = r#"
        local huly = require("assay.huly")
        local issue = huly.ensure_issue(c, { _id = "p1", identifier = "TSK" }, { title = "New one" })
        assert.eq(issue.identifier, "TSK-3")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_huly_set_issue_status() {
    let server = MockServer::start().await;
    mock_tx_ok(&server).await;

    let body = r#"
        local huly = require("assay.huly")
        local ok = huly.set_issue_status(c, { _id = "i1", space = "p1" }, "tracker:status:Done")
        assert.eq(ok, true)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();

    let tx = sent_txs(&server).await.remove(0);
    assert_eq!(tx["_class"], "core:class:TxUpdateDoc");
    assert_eq!(tx["objectId"], "i1");
    assert_eq!(tx["objectSpace"], "p1");
    assert_eq!(tx["operations"]["status"], "tracker:status:Done");
}

#[tokio::test]
async fn test_huly_statuses_filtered_to_issue_attribute() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(find_path()))
        .and(query_param("class", "tracker:class:IssueStatus"))
        .and(query_param(
            "query",
            r#"{"ofAttribute":"tracker:attribute:IssueStatus"}"#,
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(total_array(
            serde_json::json!([
                {"_id": "tracker:status:Backlog", "name": "Backlog"},
                {"_id": "tracker:status:Done", "name": "Done"},
            ]),
            2,
        )))
        .mount(&server)
        .await;

    let body = r#"
        local huly = require("assay.huly")
        local st = huly.statuses(c)
        assert.eq(#st, 2)
        assert.eq(st[2].name, "Done")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_huly_create_component_and_milestone() {
    let server = MockServer::start().await;
    mock_tx_ok(&server).await;
    mock_find(
        &server,
        "tracker:class:Component",
        serde_json::json!([{"_id": "c1", "label": "Infra"}]),
    )
    .await;
    mock_find(
        &server,
        "tracker:class:Milestone",
        serde_json::json!([{"_id": "m1", "label": "v1"}]),
    )
    .await;

    let body = r#"
        local huly = require("assay.huly")
        local proj = { _id = "p1", identifier = "TSK" }
        assert.eq(huly.create_component(c, proj, { label = "Infra" }).label, "Infra")
        assert.eq(huly.create_milestone(c, proj, { label = "v1", targetDate = 123 }).label, "v1")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();

    let txs = sent_txs(&server).await;
    assert_eq!(txs.len(), 2);
    assert_eq!(txs[0]["objectClass"], "tracker:class:Component");
    assert_eq!(txs[0]["attributes"]["comments"], 0);
    assert_eq!(txs[1]["objectClass"], "tracker:class:Milestone");
    assert_eq!(txs[1]["attributes"]["targetDate"], 123);
    assert_eq!(txs[1]["attributes"]["status"], 0);
}
