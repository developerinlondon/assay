//! assay.salesforge against the shapes probed live on 2026-09-04, anonymised.
//! What is pinned: the public key riding bare in `Authorization` with no Bearer
//! prefix, an empty list arriving as a JSON object rather than an array, paging
//! by limit and offset, the public API carrying no warm-up state while the
//! internal one does, a failed sign-in reading as a typed sign_in error that
//! leaves the public API working, and 401, 402 and 429 reading as themselves.
//! Webhooks are the one family with no REST route at any version, so they go
//! over the MCP endpoint and are pinned here as such.

#[path = "../common/mod.rs"]
mod common;

use common::run_lua;
use serde_json::json;
use wiremock::matchers::{body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const WS: &str = "wks_xa1";

/// One server stands in for all three endpoints; the paths do not collide.
fn client(uri: &str, extra: &str) -> String {
    format!(
        "local sf = require(\"assay.salesforge\")\n\
         local c = sf.client({{ api_key = \"k\", workspace_id = \"{WS}\",\n\
           base_url = \"{uri}/public/v2\", internal_base_url = \"{uri}\",\n\
           identity_url = \"{uri}/identity\"{extra} }})\n"
    )
}

fn page(rows: serde_json::Value, total: u64) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_json(json!({ "data": rows, "total": total, "limit": 100, "offset": 0 }))
}

fn public_row() -> serde_json::Value {
    json!({
        "id": "mbx_xa1", "address": "Ada@Example.TEST", "firstName": "Ada", "lastName": "Person",
        "status": "ACTIVE", "dailyEmailLimit": 30, "mailboxProvider": "GOOGLE",
        "trackingDomainStatus": "NOT_SET",
    })
}

fn internal_row(activated: bool) -> serde_json::Value {
    json!({
        "id": "mbx_xa1", "address": "ada@example.test", "status": "active",
        "warmupActivated": activated, "daysUntilWarm": 5, "dailyEmailLimit": 30,
        "mailboxProvider": "GOOGLE", "sentEmailsToday": 4, "workspaceId": WS,
    })
}

async fn mount_public(server: &MockServer, route: &str, rows: serde_json::Value, total: u64) {
    Mock::given(method("GET"))
        // The key is an apiKey scheme, not a bearer one. A client that prefixed
        // it with "Bearer " would never match this mock.
        .and(header("authorization", "k"))
        .and(path(format!("/public/v2{route}")))
        .respond_with(page(rows, total))
        .mount(server)
        .await;
}

async fn mount_sign_in(server: &MockServer, status: u16, body: serde_json::Value) {
    Mock::given(method("POST"))
        .and(path("/identity"))
        .and(query_param(
            "key",
            "AIzaSyCSvPu4xQeXnowWbgt2uRFGwAuMhkbJo-o",
        ))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

/// The public API lists what the workspace is connected to and carries no
/// warm-up state at all — no `warmupActivated`, no days, no score.
#[tokio::test]
async fn test_the_public_key_rides_bare_in_authorization_with_no_bearer_prefix() {
    let server = MockServer::start().await;
    mount_public(
        &server,
        &format!("/workspaces/{WS}/mailboxes"),
        json!([public_row()]),
        1,
    )
    .await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        local rows = c:mailboxes()
        assert.eq(#rows, 1)
        assert.eq(rows[1].address, "ada@example.test")
        assert.eq(rows[1].domain, "example.test")
        assert.eq(rows[1].status, "active")
        assert.eq(rows[1].provider_ref, "mbx_xa1")
        assert.eq(rows[1].daily_limit, 30)
        assert.eq(rows[1].connected, true)
        assert.eq(rows[1].warmup, nil)
        "#
    ))
    .await
    .unwrap();
}

/// A workspace with no sequences answers `{"data": {}, "total": 0}` — an object
/// where a list belongs. Read as an error it would look like an outage.
#[tokio::test]
async fn test_an_empty_list_arriving_as_an_object_is_an_empty_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/public/v2/workspaces/{WS}/sequences")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {}, "total": 0, "limit": 100, "offset": 0,
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        local rows, meta = c:sequences()
        assert.eq(#rows, 0)
        -- An empty answer is a real one, not a walk that got cut short.
        assert.eq(meta.truncated, false)
        assert.eq(meta.seen, 0)
        "#
    ))
    .await
    .unwrap();
}

/// The vendor pages by limit and offset. A caller that read one page would
/// report a workspace smaller than it is.
#[tokio::test]
async fn test_paging_walks_by_offset_and_stops_on_the_total() {
    let server = MockServer::start().await;
    let full: Vec<serde_json::Value> = (0..100)
        .map(|n| json!({ "id": format!("mbx_x{n}"), "address": format!("p{n}@example.test") }))
        .collect();
    Mock::given(method("GET"))
        .and(path(format!("/public/v2/workspaces/{WS}/mailboxes")))
        .and(query_param("offset", "0"))
        .respond_with(page(json!(full), 101))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/public/v2/workspaces/{WS}/mailboxes")))
        .and(query_param("offset", "100"))
        .respond_with(page(
            json!([{ "id": "mbx_xlast", "address": "last@example.test" }]),
            101,
        ))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        local rows = c:mailboxes()
        assert.eq(#rows, 101)
        assert.eq(rows[101].address, "last@example.test")
        "#
    ))
    .await
    .unwrap();
}

/// The warm-up state lives only on the web app's own API, reached with a
/// Firebase id token as a bearer. The public key does not open it.
#[tokio::test]
async fn test_the_internal_api_carries_the_warmup_state_behind_a_bearer_id_token() {
    let server = MockServer::start().await;
    mount_sign_in(
        &server,
        200,
        json!({ "idToken": "tok_x1", "expiresIn": "3600" }),
    )
    .await;
    Mock::given(method("GET"))
        .and(path(format!("/workspaces/{WS}/mailboxes")))
        .and(header("authorization", "Bearer tok_x1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [internal_row(true), internal_row(false)],
            "pagination": { "currentPage": 1, "pageSize": 50, "totalItems": 2,
                            "totalPages": 1, "next": "", "previous": "" },
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        client(
            &server.uri(),
            ", email = \"ops@example.test\", password = \"pw\""
        ),
        r#"
        local rows = c:mailboxes_internal()
        assert.eq(#rows, 2)
        assert.eq(rows[1].warmup.activated, true)
        assert.eq(rows[1].warmup.days_until_warm, 5)
        -- No reputationScore on the live row: an absent heat is a reading
        -- nobody has yet, never a heat of zero.
        assert.eq(rows[1].warmup.heat, nil)
        -- The switch is off, so there is no curve to be part-way along.
        assert.eq(rows[2].warmup, nil)
        "#
    ))
    .await
    .unwrap();
}

/// Sign-in happens once for the life of the client, however many internal calls
/// follow it.
#[tokio::test]
async fn test_sign_in_is_memoised_and_the_token_is_never_returned() {
    let server = MockServer::start().await;
    mount_sign_in(&server, 200, json!({ "idToken": "tok_x1" })).await;
    Mock::given(method("GET"))
        .and(path(format!("/workspaces/{WS}/mailboxes")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [internal_row(true)],
            "pagination": { "totalPages": 1, "next": "" },
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        client(
            &server.uri(),
            ", email = \"ops@example.test\", password = \"pw\""
        ),
        r#"
        assert.eq(c:sign_in(), true)
        assert.eq(c:sign_in(), true)
        c:mailboxes_internal()
        c:mailboxes_internal()
        "#
    ))
    .await
    .unwrap();
    let signins = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/identity")
        .count();
    assert_eq!(signins, 1, "signed in more than once");
}

/// The two surfaces authenticate differently, so an account that cannot sign in
/// must not take the public API down with it.
#[tokio::test]
async fn test_a_failed_sign_in_is_typed_and_leaves_the_public_api_working() {
    let server = MockServer::start().await;
    mount_sign_in(
        &server,
        400,
        json!({ "error": { "code": 400, "message": "INVALID_LOGIN_CREDENTIALS" } }),
    )
    .await;
    mount_public(
        &server,
        &format!("/workspaces/{WS}/mailboxes"),
        json!([public_row()]),
        1,
    )
    .await;
    run_lua(&format!(
        "{}{}",
        client(
            &server.uri(),
            ", email = \"ops@example.test\", password = \"wrong\""
        ),
        r#"
        local rows, err = c:mailboxes_internal()
        assert.eq(rows, nil)
        assert.eq(err.code, "sign_in")
        assert.eq(err.status, 400)
        assert.contains(tostring(err), "salesforge: ")
        -- The public key is untouched by the account's failure.
        assert.eq(#c:mailboxes(), 1)
        "#
    ))
    .await
    .unwrap();
}

/// A client with no account at all reports the same typed error rather than
/// reaching for a credential nobody supplied.
#[tokio::test]
async fn test_an_absent_account_is_a_sign_in_error_not_a_crash() {
    let server = MockServer::start().await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ", email = \"\", password = \"\""),
        r#"
        local rows, err = c:mailboxes_internal()
        assert.eq(rows, nil)
        assert.eq(err.code, "sign_in")
        "#
    ))
    .await
    .unwrap();
}

/// A refused key is not an empty workspace, and the plan gate is not a bad
/// request: the public API is Growth-only and answers 402 when it is not.
#[tokio::test]
async fn test_auth_plan_and_rate_limits_read_as_themselves() {
    // Every verb the client uses, not just the reads.
    //
    // A refused write is the case worth driving: if the HTTP builtin threw on a
    // 4xx PUT instead of answering with one, the client's `pcall` would catch
    // it and report `code = "transport"` with no status at all. Asserting the
    // status-derived code and the status itself is what tells those two apart,
    // so a vendor saying "no" stays legible as the "no" it is.
    let calls: [(&str, String, &str); 4] = [
        (
            "GET",
            format!("/public/v2/workspaces/{WS}/mailboxes"),
            "c:mailboxes()",
        ),
        (
            "PUT",
            format!("/public/v2/workspaces/{WS}/sequences/seq_xa1/mailboxes"),
            r#"c:set_rotation("seq_xa1", { "mbx_xa1" })"#,
        ),
        (
            "PUT",
            format!("/public/v2/workspaces/{WS}/sequences/seq_xa1/status"),
            r#"c:set_sequence_status("seq_xa1", "paused")"#,
        ),
        (
            "PUT",
            format!("/public/v2/workspaces/{WS}/sequences/seq_xa1/contacts"),
            r#"c:enrol("seq_xa1", { "con_xa1" })"#,
        ),
    ];
    for (status, code) in [
        (401u16, "auth"),
        (403, "auth"),
        (402, "plan"),
        (429, "rate_limit"),
        (500, "server"),
    ] {
        for (verb, route, call) in &calls {
            let server = MockServer::start().await;
            Mock::given(method(*verb))
                .and(path(route.clone()))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            let check = format!(
                r#"
                local out, err = {call}
                assert.eq(out, nil)
                assert.eq(err.code, "{code}")
                assert.eq(err.status, {status})
                assert.contains(tostring(err), "salesforge: ")
                "#
            );
            run_lua(&format!("{}{check}", client(&server.uri(), "")))
                .await
                .unwrap_or_else(|e| panic!("{verb} {route} on {status}: {e}"));
        }
    }
}

/// Enrolling, do-not-contact and replying answer 204 with no body at all. A
/// blanket parse would turn every success into a read error.
#[tokio::test]
async fn test_the_write_calls_send_the_vendors_own_field_names_and_accept_a_204() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(format!(
            "/public/v2/workspaces/{WS}/sequences/seq_xa1/contacts"
        )))
        .and(body_string_contains("contactIds"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/public/v2/workspaces/{WS}/dnc/bulk")))
        .and(body_string_contains("dncs"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/public/v2/workspaces/{WS}/mailboxes/mbx_xa1/emails/em_xa9/reply"
        )))
        .and(body_string_contains("includeHistory"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/public/v2/workspaces/{WS}/contacts")))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "con_xa1", "email": "ada@example.test",
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        assert.eq(c:enrol("seq_xa1", { "con_xa1" }), true)
        assert.eq(c:dnc({ "ada@example.test" }), true)
        assert.eq(c:reply("mbx_xa1", "em_xa9", "thanks"), true)
        assert.eq(c:create_contact({ firstName = "Ada", email = "ada@example.test" }).id, "con_xa1")
        "#
    ))
    .await
    .unwrap();
}

/// An empty list would encode as a JSON object rather than an empty array and
/// reach the vendor as a malformed body, so it never leaves this process.
#[tokio::test]
async fn test_an_empty_write_is_refused_here_rather_than_sent_malformed() {
    let server = MockServer::start().await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        local ok, err = c:enrol("seq_xa1", {})
        assert.eq(ok, nil)
        assert.eq(err.code, "config")
        local ok2, err2 = c:dnc({})
        assert.eq(ok2, nil)
        assert.eq(err2.code, "config")
        local ok3, err3 = c:create_contact({ email = "ada@example.test" })
        assert.eq(ok3, nil)
        assert.eq(err3.code, "config")
        "#
    ))
    .await
    .unwrap();
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "a malformed write reached the vendor"
    );
}

/// A client with no key or no workspace is a programming error, not a vendor
/// answer.
#[tokio::test]
async fn test_a_client_refuses_to_build_without_a_key_or_a_workspace() {
    for args in [r#"{ workspace_id = "wks_xa1" }"#, r#"{ api_key = "k" }"#] {
        let err = run_lua(&format!(
            "local sf = require(\"assay.salesforge\")\nsf.client({args})"
        ))
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("required"), "gave {err}");
    }
}

/// A walk the vendor ended by running out of rows has the whole workspace.
#[tokio::test]
async fn test_a_walk_the_vendor_ended_is_not_truncated() {
    let server = MockServer::start().await;
    mount_public(
        &server,
        &format!("/workspaces/{WS}/mailboxes"),
        json!([public_row()]),
        1,
    )
    .await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        local rows, meta = c:mailboxes()
        assert.eq(#rows, 1)
        assert.eq(meta.truncated, false)
        assert.eq(meta.cap, 2000)
        assert.eq(meta.seen, 1)
        "#
    ))
    .await
    .unwrap();
}

/// A vendor answering full pages against a total it never reaches walks into
/// the page cap. Read without `meta` that is a workspace two thousand mailboxes
/// wide reported as if it were the whole thing.
#[tokio::test]
async fn test_a_public_walk_stopped_by_the_page_cap_says_it_is_truncated() {
    let server = MockServer::start().await;
    let full: Vec<serde_json::Value> = (0..100)
        .map(|n| json!({ "id": format!("mbx_x{n}"), "address": format!("p{n}@example.test") }))
        .collect();
    Mock::given(method("GET"))
        .and(path(format!("/public/v2/workspaces/{WS}/mailboxes")))
        .respond_with(page(json!(full), 999_999))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        local rows, meta = c:mailboxes()
        assert.eq(meta.truncated, true)
        assert.eq(meta.cap, 2000)
        assert.eq(meta.seen, 2000)
        assert.eq(#rows, 2000)
        "#
    ))
    .await
    .unwrap();
}

/// The internal API pages by a `next` link, so a vendor that always offers one
/// walks into the same cap. Its warm-up list can be short for the same reason.
#[tokio::test]
async fn test_an_internal_walk_stopped_by_the_page_cap_says_it_is_truncated() {
    let server = MockServer::start().await;
    mount_sign_in(&server, 200, json!({ "idToken": "tok_x1" })).await;
    let full: Vec<serde_json::Value> = (0..50)
        .map(|n| {
            json!({ "id": format!("mbx_x{n}"), "address": format!("p{n}@example.test"),
                    "warmupActivated": true, "daysUntilWarm": 5 })
        })
        .collect();
    Mock::given(method("GET"))
        .and(path(format!("/workspaces/{WS}/mailboxes")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": full,
            "pagination": { "currentPage": 1, "pageSize": 50, "next": "?page=2&size=50" },
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        client(
            &server.uri(),
            ", email = \"ops@example.test\", password = \"pw\""
        ),
        r#"
        local rows, meta = c:mailboxes_internal()
        assert.eq(meta.truncated, true)
        assert.eq(meta.cap, 1000)
        assert.eq(meta.seen, 1000)
        "#
    ))
    .await
    .unwrap();
}

/// Every environment variable a module reads has to reach `assay context`, and
/// a quickref only gets there if it parses.
///
/// `parse_quickref` wants `signature -> return_hint | description` and silently
/// drops anything else, so a malformed line is not a formatting nit: the method
/// or the variable it documents simply never appears. Two lines were lost that
/// way before this test existed.
#[tokio::test]
async fn test_every_quickref_parses_and_the_env_vars_are_among_them() {
    let modules = ["clayinbox", "forge", "salesforge"];
    let mut salesforge_refs = String::new();
    for name in modules {
        let path = format!("{}/stdlib/{name}.lua", env!("CARGO_MANIFEST_DIR"));
        let source = std::fs::read_to_string(&path).unwrap();
        let mut parsed = 0;
        for line in source.lines().take_while(|l| l.starts_with("---")) {
            let Some(value) = line.strip_prefix("--- @quickref ") else {
                continue;
            };
            // The parser's own rule, mirrored: split on " -> ", then on " | ".
            let rest = value
                .split_once(" -> ")
                .unwrap_or_else(|| panic!("{name}: quickref has no ` -> `: {value}"))
                .1;
            rest.split_once(" | ")
                .unwrap_or_else(|| panic!("{name}: quickref has no ` | `: {value}"));
            parsed += 1;
            if name == "salesforge" {
                salesforge_refs.push_str(value);
                salesforge_refs.push('\n');
            }
        }
        assert!(parsed > 0, "{name} declares no quickrefs at all");
    }
    for var in [
        "SALESFORGE_API_KEY",
        "SALESFORGE_EMAIL",
        "SALESFORGE_PASSWORD",
    ] {
        assert!(
            salesforge_refs.contains(var),
            "{var} is read but no parsing quickref names it, so `assay context` never shows it"
        );
    }
}

/// The rotation is replaced wholesale: a caller taking one domain out sends
/// back the ids it means to keep, under the vendor's own `mailboxIds` key.
#[tokio::test]
async fn test_setting_a_rotation_sends_the_vendors_own_key_and_accepts_a_204() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(header("authorization", "k"))
        .and(path(format!(
            "/public/v2/workspaces/{WS}/sequences/seq_xa1/mailboxes"
        )))
        .and(body_string_contains("mailboxIds"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        assert.eq(c:set_rotation("seq_xa1", { "mbx_xa1", "mbx_xa2" }), true)
        "#
    ))
    .await
    .unwrap();
    let sent = &server.received_requests().await.unwrap()[0];
    let body = String::from_utf8(sent.body.clone()).unwrap();
    assert_eq!(
        body, r#"{"mailboxIds":["mbx_xa1","mbx_xa2"]}"#,
        "rotation body was {body}"
    );
}

/// The vendor takes two statuses. Constraining them here makes a typo a config
/// error the caller can read rather than a 400 it has to interpret.
#[tokio::test]
async fn test_a_sequence_status_is_one_of_two_words_and_a_typo_never_leaves_the_process() {
    let server = MockServer::start().await;
    for state in ["paused", "active"] {
        Mock::given(method("PUT"))
            .and(header("authorization", "k"))
            .and(path(format!(
                "/public/v2/workspaces/{WS}/sequences/seq_xa1/status"
            )))
            .and(body_string_contains(state))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
    }
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        assert.eq(c:set_sequence_status("seq_xa1", "paused"), true)
        assert.eq(c:set_sequence_status("seq_xa1", "active"), true)
        -- Case is the caller's business, not the vendor's.
        assert.eq(c:set_sequence_status("seq_xa1", "PAUSED"), true)
        local ok, err = c:set_sequence_status("seq_xa1", "pasued")
        assert.eq(ok, nil)
        assert.eq(err.code, "config")
        assert.contains(err.message, "paused")
        local ok2, err2 = c:set_sequence_status("seq_xa1", nil)
        assert.eq(ok2, nil)
        assert.eq(err2.code, "config")
        "#
    ))
    .await
    .unwrap();
    // Three good calls reached the vendor; neither bad one did.
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
}

/// Clearing a rotation is a real instruction, not a malformed one.
///
/// Pulling a paused domain's boxes can legitimately leave a sequence with none,
/// and that is the truthful state — it then cannot send, which is what
/// `set_sequence_status` is for. The caller must be able to say it rather than
/// being forced to leave a stale mailbox in the rotation.
///
/// The body is the whole point: a bare empty Lua table encodes as `{}`, which
/// the vendor reads as a malformed object, so the ids ride a table marked as a
/// JSON array and the wire form is `[]`.
#[tokio::test]
async fn test_an_empty_rotation_clears_it_and_sends_an_array_not_an_object() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(format!(
            "/public/v2/workspaces/{WS}/sequences/seq_xa1/mailboxes"
        )))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        assert.eq(c:set_rotation("seq_xa1", {}), true)
        "#
    ))
    .await
    .unwrap();
    let sent = &server.received_requests().await.unwrap()[0];
    let body = String::from_utf8(sent.body.clone()).unwrap();
    assert_eq!(
        body, r#"{"mailboxIds":[]}"#,
        "empty rotation body was {body}"
    );
}

/// The caller's own table must come back unmarked: the array marker rides a
/// copy, so passing the same list to something else afterwards is unaffected.
#[tokio::test]
async fn test_the_array_marker_does_not_touch_the_callers_table() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(format!(
            "/public/v2/workspaces/{WS}/sequences/seq_xa1/mailboxes"
        )))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        local ids = {}
        assert.eq(c:set_rotation("seq_xa1", ids), true)
        assert.eq(getmetatable(ids), nil)
        "#
    ))
    .await
    .unwrap();
}

/// A blank sequence id would address the workspace itself, and a non-table is
/// not a list at all. Neither leaves this process.
#[tokio::test]
async fn test_a_blank_id_or_a_non_list_is_refused_before_any_request() {
    let server = MockServer::start().await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        local ok, err = c:set_rotation("", { "mbx_xa1" })
        assert.eq(ok, nil)
        assert.eq(err.code, "config")
        local ok2, err2 = c:set_rotation("seq_xa1", "mbx_xa1")
        assert.eq(ok2, nil)
        assert.eq(err2.code, "config")
        local ok3, err3 = c:set_rotation("seq_xa1", nil)
        assert.eq(ok3, nil)
        assert.eq(err3.code, "config")
        local ok4, err4 = c:set_sequence_status("", "paused")
        assert.eq(ok4, nil)
        assert.eq(err4.code, "config")
        "#
    ))
    .await
    .unwrap();
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "a malformed sequence write reached the vendor"
    );
}

const CREDS: &str = ", email = \"ops@example.test\", password = \"pw\"";

/// The shape the web app's own `/me` answers: the plan is nested two deep, under
/// the user's account, and the credit pools sit beside it on the account rather
/// than on the plan. A trial states no billing cycle anywhere.
fn me_body() -> serde_json::Value {
    json!({
        "user": {
            "id": "usr_xa1", "firstName": "Ada", "lastName": "Person",
            "email": "ops@example.test", "accountId": "acc_xa1",
            "account": {
                "id": "acc_xa1", "name": "Example Ltd",
                "activePlanId": "plan_trial",
                "activePlan": {
                    "id": "plan_trial", "name": "Trial",
                    "activatedLeadsPerMonthLimit": 50, "emailsPerMonthLimit": 100,
                    "validationsPerMonthLimit": 50, "personalizationsPerMonthLimit": 50,
                    "socialActionsPerMonthLimit": 50, "linkedInProfilesLimit": 1,
                },
                "planStartedAt": "2026-08-31T09:12:48.185965Z",
                "subscriptionStatus": "active",
                "freeTrialExpiresAt": "2026-09-14T09:12:48.185965Z",
                "emailCreditsLeft": 100, "leadCreditsLeft": 50,
                "emailValidationCreditsLeft": 50, "personalizationCreditsLeft": 50,
                "socialActionCreditsLeft": 50,
                "StripeCustomerID": serde_json::Value::Null,
            },
        },
    })
}

async fn mount_me(server: &MockServer, status: u16, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/me"))
        .and(header("authorization", "Bearer tok_x1"))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

/// The plan and its ceilings live only on the internal `/me`. Every plan,
/// billing, usage and limits path under the public workspace is a flat 404, and
/// the internal subscription route holds nothing for an account that never
/// bought one.
#[tokio::test]
async fn test_the_plan_and_its_limits_come_off_the_internal_me_endpoint() {
    let server = MockServer::start().await;
    mount_sign_in(&server, 200, json!({ "idToken": "tok_x1" })).await;
    mount_me(&server, 200, me_body()).await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), CREDS),
        r#"
        local out = c:costs()
        assert.eq(#out.items, 1)
        assert.eq(out.items[1].kind, "plan")
        assert.eq(out.items[1].unit, "plan")
        assert.eq(out.items[1].ref, "Trial")
        assert.eq(out.items[1].quantity, 1)
        assert.eq(out.items[1].source, "vendor")
        assert.eq(out.meta.plan.id, "plan_trial")
        assert.eq(out.meta.plan.status, "active")
        assert.eq(out.meta.plan.trial_expires_at, "2026-09-14T09:12:48.185965Z")
        assert.eq(out.meta.limits.emails_per_month, 100)
        assert.eq(out.meta.limits.linkedin_profiles, 1)
        assert.eq(out.meta.credits_left.emails, 100)
        assert.eq(out.meta.credits_left.validations, 50)
        "#
    ))
    .await
    .unwrap();
}

/// The vendor names no money anywhere on the plan it says you are on. An absent
/// price read as free would put the sequencer's cost at nothing, so the item
/// carries no price and `meta.priced` says why.
#[tokio::test]
async fn test_the_vendor_names_no_price_so_costs_says_so_rather_than_reading_as_free() {
    let server = MockServer::start().await;
    mount_sign_in(&server, 200, json!({ "idToken": "tok_x1" })).await;
    mount_me(&server, 200, me_body()).await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), CREDS),
        r#"
        local out = c:costs()
        assert.eq(out.items[1].unit_price_cents, nil)
        assert.eq(out.items[1].currency, nil)
        assert.eq(out.meta.priced, false)
        assert.eq(out.meta.currency_known, false)
        "#
    ))
    .await
    .unwrap();
}

/// A trial states no billing cycle in any field. A period of "month" here would
/// be this module's guess wearing the vendor's name, so there is none — the
/// same way the absent price is absent rather than zero.
#[tokio::test]
async fn test_a_plan_whose_cycle_the_vendor_never_states_carries_no_period() {
    let server = MockServer::start().await;
    mount_sign_in(&server, 200, json!({ "idToken": "tok_x1" })).await;
    mount_me(&server, 200, me_body()).await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), CREDS),
        r#"
        local out = c:costs()
        assert.eq(out.items[1].period, nil)
        -- The ceilings stay monthly whatever the plan bills on; that is the
        -- vendor's own field naming, not a cycle.
        assert.eq(out.meta.limits.emails_per_month, 100)
        "#
    ))
    .await
    .unwrap();
}

/// An annual plan reads as a year. The vendor writes the cycle on the plan on
/// some accounts and on the account on others, so all three fields are read.
#[tokio::test]
async fn test_an_annual_plan_reads_as_a_year_wherever_the_vendor_states_it() {
    let cases = [
        json!({ "activePlanId": "plan_growth",
                "activePlan": { "name": "Growth", "interval": "year" } }),
        json!({ "activePlanId": "plan_growth",
                "activePlan": { "name": "Growth", "billingPeriod": "YEARLY" } }),
        json!({ "activePlanId": "plan_growth", "billingCycle": "ANNUAL",
                "activePlan": { "name": "Growth" } }),
    ];
    for account in cases {
        let server = MockServer::start().await;
        mount_sign_in(&server, 200, json!({ "idToken": "tok_x1" })).await;
        mount_me(&server, 200, json!({ "user": { "account": account } })).await;
        run_lua(&format!(
            "{}{}",
            client(&server.uri(), CREDS),
            r#"
            local out = c:costs()
            assert.eq(out.items[1].period, "year")
            assert.eq(out.items[1].ref, "Growth")
            "#
        ))
        .await
        .unwrap();
    }
}

/// A monthly plan reads as a month when the vendor is the one saying so.
#[tokio::test]
async fn test_a_monthly_plan_reads_as_a_month_when_the_vendor_states_it() {
    let server = MockServer::start().await;
    mount_sign_in(&server, 200, json!({ "idToken": "tok_x1" })).await;
    mount_me(
        &server,
        200,
        json!({ "user": { "account": {
            "activePlanId": "plan_growth", "billingCycle": "MONTHLY",
            "activePlan": { "name": "Growth" },
        } } }),
    )
    .await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), CREDS),
        r#"
        local out = c:costs()
        assert.eq(out.items[1].period, "month")
        "#
    ))
    .await
    .unwrap();
}

/// A `/me` the vendor refuses reads as a refusal. Answering a plan of "unknown"
/// with empty limits would look like an account entitled to nothing.
#[tokio::test]
async fn test_a_refused_me_reads_as_an_error_not_as_an_account_entitled_to_nothing() {
    for (status, code) in [
        (401u16, "auth"),
        (402, "plan"),
        (429, "rate_limit"),
        (500, "server"),
    ] {
        let server = MockServer::start().await;
        mount_sign_in(&server, 200, json!({ "idToken": "tok_x1" })).await;
        mount_me(&server, status, json!({ "message": "no" })).await;
        let body = format!(
            r#"
            local out, err = c:costs()
            assert.eq(out, nil)
            assert.eq(err.code, "{code}")
            assert.eq(err.status, {status})
            "#
        );
        run_lua(&format!("{}{}", client(&server.uri(), CREDS), body))
            .await
            .unwrap();
    }
}

/// A 200 carrying nothing, a bare JSON scalar and a JSON `null` all arrive as
/// something that is not an account. Indexed they crash the caller; read as an
/// empty account they report a workspace entitled to nothing, which is a plan
/// downgrade that never happened.
#[tokio::test]
async fn test_a_me_that_is_not_an_account_object_is_a_typed_read_error() {
    let bodies = ["", "true", "null", "\"nope\"", "[]"];
    for raw in bodies {
        let server = MockServer::start().await;
        mount_sign_in(&server, 200, json!({ "idToken": "tok_x1" })).await;
        Mock::given(method("GET"))
            .and(path("/me"))
            .respond_with(ResponseTemplate::new(200).set_body_string(raw))
            .mount(&server)
            .await;
        run_lua(&format!(
            "{}{}",
            client(&server.uri(), CREDS),
            r#"
            local out, err = c:costs()
            assert.eq(out, nil)
            assert.eq(type(err), "table")
            assert.contains(tostring(err), "salesforge: ")
            "#
        ))
        .await
        .unwrap();
    }
}

/// Costing needs the internal API, so a sign-in that fails stops it with the
/// sign-in error rather than a half-read plan.
#[tokio::test]
async fn test_costs_without_a_sign_in_is_a_typed_sign_in_error() {
    let server = MockServer::start().await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        local out, err = c:costs()
        assert.eq(out, nil)
        assert.eq(err.code, "sign_in")
        "#
    ))
    .await
    .unwrap();
}

/// An account the vendor answers without a plan object is an account whose plan
/// this module could not read. It names the plan id it does have rather than
/// inventing limits.
#[tokio::test]
async fn test_an_account_with_no_plan_object_falls_back_to_the_plan_id() {
    let server = MockServer::start().await;
    mount_sign_in(&server, 200, json!({ "idToken": "tok_x1" })).await;
    mount_me(
        &server,
        200,
        json!({ "user": { "account": { "activePlanId": "plan_growth" } } }),
    )
    .await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), CREDS),
        r#"
        local out = c:costs()
        assert.eq(out.items[1].ref, "plan_growth")
        assert.eq(out.items[1].period, nil)
        assert.eq(out.meta.limits.emails_per_month, nil)
        assert.eq(out.meta.credits_left.emails, nil)
        "#
    ))
    .await
    .unwrap();
}

// ─── connecting a mailbox, and switching its warm-up on ────────────────────

const INTERNAL_AUTH: &str = "Bearer tok_x1";

async fn mount_internal_listing(server: &MockServer, rows: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!("/workspaces/{WS}/mailboxes")))
        .and(header("authorization", INTERNAL_AUTH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": rows,
            "pagination": { "totalPages": 1, "next": "" },
        })))
        .mount(server)
        .await;
}

fn account(uri: &str) -> String {
    client(uri, ", email = \"ops@example.test\", password = \"pw\"")
}

/// The body the vendor documents, and the only shape it accepts: one password
/// carried into both transport blocks, the address as the username on each.
#[tokio::test]
async fn test_connect_smtp_sends_the_transport_blocks_the_vendor_asks_for() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/public/v2/workspaces/{WS}/mailboxes")))
        .and(header("authorization", "k"))
        .and(body_string_contains("\"address\":\"ada@example.test\""))
        .and(body_string_contains("\"host\":\"smtp.gmail.com\""))
        .and(body_string_contains("\"port\":587"))
        .and(body_string_contains("\"host\":\"imap.gmail.com\""))
        .and(body_string_contains("\"port\":993"))
        .and(body_string_contains("\"username\":\"ada@example.test\""))
        .and(body_string_contains("\"password\":\"app-pw\""))
        .and(body_string_contains("\"firstName\":\"Ada\""))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "mbx_new", "address": "ada@example.test", "status": "PENDING",
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        local box = c:connect_smtp("Ada@Example.TEST", "app-pw", { first = "Ada", last = "Person" })
        assert.eq(box.address, "ada@example.test")
        assert.eq(box.provider_ref, "mbx_new")
        assert.eq(box.status, "pending")
        -- The vendor verifies the credentials afterwards, so a box it has only
        -- accepted is not a box anything can send from yet.
        assert.eq(box.connected, false)
        "#
    ))
    .await
    .unwrap();
}

/// The password this call sends must never come back out of it.
///
/// `raw` is handed back whole so a caller can read metadata this module does
/// not map. On the connect path that whole is the answer to a body containing
/// the mailbox password, so a vendor that echoed the transport blocks — which
/// it does not today — would put a plaintext credential into every caller that
/// prints a row. The keys come off rather than being trusted not to appear.
#[tokio::test]
async fn test_a_create_response_that_echoes_the_transport_blocks_carries_no_password_out() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/public/v2/workspaces/{WS}/mailboxes")))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "mbx_new",
            "address": "ada@example.test",
            "status": "ACTIVE",
            "dailyEmailLimit": 30,
            // The hypothetical: the vendor hands the submitted blocks back.
            "smtp": { "host": "smtp.gmail.com", "port": 587,
                      "username": "ada@example.test", "password": "app-pw-secret" },
            "imap": { "host": "imap.gmail.com", "port": 993,
                      "username": "ada@example.test", "password": "app-pw-secret" },
            "password": "app-pw-secret",
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        local box = c:connect_smtp("ada@example.test", "app-pw-secret")
        assert.eq(box.address, "ada@example.test")
        assert.eq(box.connected, true)
        -- Nothing anywhere in the returned table, however a caller walks it.
        assert.eq(box.raw.smtp, nil)
        assert.eq(box.raw.imap, nil)
        assert.eq(box.raw.password, nil)
        assert.contains(json.encode(box), "mbx_new")
        assert.eq(json.encode(box):find("app-pw-secret", 1, true), nil)
        -- The metadata `raw` exists for is still there.
        assert.eq(box.raw.dailyEmailLimit, 30)
        "#
    ))
    .await
    .unwrap();
}

/// A vendor that already says `active` is the one case where the box IS wired.
#[tokio::test]
async fn test_connect_smtp_reports_connected_only_when_the_vendor_already_says_active() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/public/v2/workspaces/{WS}/mailboxes")))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "mbx_new", "address": "ada@example.test", "status": "ACTIVE",
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        local box = c:connect_smtp("ada@example.test", "app-pw")
        assert.eq(box.status, "active")
        assert.eq(box.connected, true)
        -- No name given: the local part beats a refused request, and the vendor
        -- rejects a body with no first name at all.
        "#
    ))
    .await
    .unwrap();
}

/// The vendor answers 2xx with its own refusal in the body when the credentials
/// do not verify. Read as a created mailbox that is a box nothing can send
/// from, reported as connected.
#[tokio::test]
async fn test_a_two_hundred_carrying_a_refusal_is_an_error_and_never_a_mailbox() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/public/v2/workspaces/{WS}/mailboxes")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": "failed to verify mailbox credentials",
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        local box, err = c:connect_smtp("ada@example.test", "dead-pw")
        assert.eq(box, nil)
        assert.eq(err.code, "refused")
        assert.contains(err.message, "failed to verify mailbox credentials")
        "#
    ))
    .await
    .unwrap();
}

/// Refused here rather than at the vendor: a call with no password would reach
/// the API as a mailbox connected to nothing.
#[tokio::test]
async fn test_connect_smtp_refuses_a_bad_address_or_an_empty_password_before_calling() {
    let server = MockServer::start().await;
    run_lua(&format!(
        "{}{}",
        client(&server.uri(), ""),
        r#"
        local _, no_at = c:connect_smtp("not-an-address", "pw")
        assert.eq(no_at.code, "config")
        local _, no_pw = c:connect_smtp("ada@example.test", "   ")
        assert.eq(no_pw.code, "config")
        "#
    ))
    .await
    .unwrap();
    // Nothing was sent: an unmounted server answers 404, and either call would
    // have come back as an http error rather than a config one.
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// The read-back is the whole point. The PUT answers exactly what it was asked
/// for; only the GET afterwards says what the vendor actually holds.
#[tokio::test]
async fn test_set_warmup_reports_what_the_vendor_holds_and_not_what_it_was_asked_for() {
    let server = MockServer::start().await;
    mount_sign_in(&server, 200, json!({ "idToken": "tok_x1" })).await;
    Mock::given(method("PUT"))
        .and(path(format!("/workspaces/{WS}/mailboxes/mbx_xa1")))
        .and(header("authorization", INTERNAL_AUTH))
        .and(body_string_contains("\"warmupActivated\":true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "mbx_xa1", "address": "ada@example.test", "warmupActivated": true,
        })))
        .mount(&server)
        .await;
    // …and the box the vendor then hands back still has it off.
    Mock::given(method("GET"))
        .and(path(format!("/workspaces/{WS}/mailboxes/mbx_xa1")))
        .and(header("authorization", INTERNAL_AUTH))
        .respond_with(ResponseTemplate::new(200).set_body_json(internal_row(false)))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        account(&server.uri()),
        r#"
        local box = c:set_warmup("mbx_xa1", true)
        assert.eq(box.address, "ada@example.test")
        -- Not `warmup.activated == true`: the PUT said so and the box does not.
        assert.eq(box.warmup, nil)
        "#
    ))
    .await
    .unwrap();
}

/// The switch on, read back on: days and heat come off the row the GET returned.
#[tokio::test]
async fn test_set_warmup_on_reads_the_curve_back_off_the_vendors_own_row() {
    let server = MockServer::start().await;
    mount_sign_in(&server, 200, json!({ "idToken": "tok_x1" })).await;
    Mock::given(method("PUT"))
        .and(path(format!("/workspaces/{WS}/mailboxes/mbx_xa1")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "mbx_xa1" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/workspaces/{WS}/mailboxes/mbx_xa1")))
        // Some of these routes wrap the object; both shapes read the same.
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": internal_row(true),
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        account(&server.uri()),
        r#"
        local box = c:set_warmup("mbx_xa1", true)
        assert.eq(box.warmup.activated, true)
        assert.eq(box.warmup.days_until_warm, 5)
        "#
    ))
    .await
    .unwrap();
}

/// An operator holds addresses; these endpoints take ids. The address is
/// resolved on the internal listing, which is also the only one that could have
/// answered for the warm-up.
#[tokio::test]
async fn test_set_warmup_takes_the_address_an_operator_actually_has() {
    let server = MockServer::start().await;
    mount_sign_in(&server, 200, json!({ "idToken": "tok_x1" })).await;
    mount_internal_listing(&server, json!([internal_row(false)])).await;
    Mock::given(method("PUT"))
        .and(path(format!("/workspaces/{WS}/mailboxes/mbx_xa1")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "mbx_xa1" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/workspaces/{WS}/mailboxes/mbx_xa1")))
        .respond_with(ResponseTemplate::new(200).set_body_json(internal_row(true)))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        account(&server.uri()),
        r#"
        assert.eq(c:mailbox_id("mbx_xa1"), "mbx_xa1")
        local box = c:set_warmup("Ada@Example.TEST", true)
        assert.eq(box.warmup.activated, true)
        "#
    ))
    .await
    .unwrap();
}

/// A box this workspace does not hold is a named refusal, never a PUT against
/// an address the vendor would read as an id.
#[tokio::test]
async fn test_an_address_the_workspace_does_not_hold_is_refused_by_name() {
    let server = MockServer::start().await;
    mount_sign_in(&server, 200, json!({ "idToken": "tok_x1" })).await;
    mount_internal_listing(&server, json!([internal_row(true)])).await;
    run_lua(&format!(
        "{}{}",
        account(&server.uri()),
        r#"
        local box, err = c:set_warmup("stranger@example.test", true)
        assert.eq(box, nil)
        assert.eq(err.code, "not_found")
        assert.contains(err.message, "stranger@example.test")
        local _, bad = c:set_warmup("mbx_xa1", "yes")
        assert.eq(bad.code, "config")
        "#
    ))
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// Webhooks. The vendor's REST API answers 404 for /webhooks at every version,
// so these three ride the MCP endpoint with the Salesforge key header. What is
// pinned: the header, one event per hook, the signing secret coming back from a
// create and never from a read, and a tool-level refusal reading as an error.
// ---------------------------------------------------------------------------

/// The tool's payload arrives as a JSON string nested in the reply's text part.
fn mcp_reply(payload: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": { "content": [{ "type": "text", "text": payload.to_string() }] },
    }))
}

async fn mount_mcp(server: &MockServer, tool: &str, payload: serde_json::Value) {
    Mock::given(method("POST"))
        .and(path("/mcp"))
        // The key rides its own product header, which is the whole of what
        // tells this endpoint which forge is calling.
        .and(header("x-salesforge-key", "k"))
        .and(body_string_contains(tool))
        .respond_with(mcp_reply(payload))
        .mount(server)
        .await;
}

fn mcp_client(uri: &str) -> String {
    client(uri, &format!(", mcp_base_url = \"{uri}/mcp\""))
}

#[tokio::test]
async fn test_webhooks_are_listed_from_the_mcp_endpoint() {
    let server = MockServer::start().await;
    mount_mcp(
        &server,
        "list_webhooks",
        json!({ "total": 1, "data": [{
            "id": "rwh_xa1", "name": "replies", "type": "email_replied",
            "url": "https://x.test/api/hooks/salesforge?token=t", "sentCount": 0,
        }] }),
    )
    .await;

    let script = format!(
        "{}local hooks = c:webhooks()\n\
         assert.eq(#hooks, 1)\n\
         assert.eq(hooks[1].id, \"rwh_xa1\")\n\
         assert.eq(hooks[1].type, \"email_replied\")",
        mcp_client(&server.uri())
    );
    run_lua(&script).await.unwrap();
}

/// An empty list is a list. A workspace that has never registered one is the
/// normal case, and it must not read as a failure.
#[tokio::test]
async fn test_no_webhooks_is_an_empty_list_and_not_an_error() {
    let server = MockServer::start().await;
    mount_mcp(&server, "list_webhooks", json!({ "total": 0, "data": [] })).await;
    let script = format!(
        "{}local hooks, meta = c:webhooks()\n\
         assert.not_nil(hooks)\n\
         assert.eq(#hooks, 0)\n\
         assert.eq(meta.truncated, false)",
        mcp_client(&server.uri())
    );
    run_lua(&script).await.unwrap();
}

/// The signing secret is returned once, by the create, and by nothing else. A
/// caller that does not store it here can never verify a delivery.
#[tokio::test]
async fn test_creating_a_webhook_returns_the_signing_secret_once() {
    let server = MockServer::start().await;
    mount_mcp(
        &server,
        "create_webhook",
        json!({
            "id": "rwh_xa2", "name": "replies", "type": "email_replied",
            "url": "https://x.test/api/hooks/salesforge?token=t",
            "sentCount": 0, "signingSecret": "whsec_abc",
        }),
    )
    .await;

    let script = format!(
        "{}local hook = c:create_webhook({{\n\
           url = \"https://x.test/api/hooks/salesforge?token=t\",\n\
           event = \"email_replied\", name = \"replies\",\n\
         }})\n\
         assert.eq(hook.id, \"rwh_xa2\")\n\
         assert.eq(hook.signingSecret, \"whsec_abc\")",
        mcp_client(&server.uri())
    );
    run_lua(&script).await.unwrap();
}

/// A read never carries the secret back, which is why the create is the only
/// chance to keep it.
#[tokio::test]
async fn test_reading_a_webhook_never_carries_the_secret() {
    let server = MockServer::start().await;
    mount_mcp(
        &server,
        "get_webhook",
        json!({
            "id": "rwh_xa2", "name": "replies", "type": "email_replied",
            "url": "https://x.test/api/hooks/salesforge?token=t", "sentCount": 3,
        }),
    )
    .await;

    let script = format!(
        "{}local hook = c:webhook(\"rwh_xa2\")\n\
         assert.eq(hook.sentCount, 3)\n\
         assert.eq(hook.signingSecret, nil)",
        mcp_client(&server.uri())
    );
    run_lua(&script).await.unwrap();
}

/// Both are the caller's mistake and neither reaches the vendor.
/// The tool answers ten and ignores limit and offset, so a workspace past ten
/// loses rows with nothing to say it did. A full window is the only signal
/// there is, and it reads as truncated rather than as a complete list of ten.
#[tokio::test]
async fn test_a_full_window_of_webhooks_reads_as_truncated() {
    let server = MockServer::start().await;
    let rows: Vec<serde_json::Value> = (0..10)
        .map(|i| {
            json!({
                "id": format!("rwh_{i}"), "name": "replies", "type": "email_replied",
                "url": "https://x.test/h?token=t", "sentCount": 0,
            })
        })
        .collect();
    mount_mcp(
        &server,
        "list_webhooks",
        json!({ "total": 10, "data": rows }),
    )
    .await;

    let script = format!(
        "{}local hooks, meta = c:webhooks()
         assert.eq(#hooks, 10)
         assert.eq(meta.truncated, true)
         assert.eq(meta.cap, 10)
         assert.eq(meta.seen, 10)",
        mcp_client(&server.uri())
    );
    run_lua(&script).await.unwrap();
}

/// Short of the cap the vendor has shown everything it holds.
#[tokio::test]
async fn test_a_short_window_of_webhooks_is_the_whole_list() {
    let server = MockServer::start().await;
    mount_mcp(
        &server,
        "list_webhooks",
        json!({ "total": 2, "data": [
            { "id": "rwh_1", "type": "email_replied", "url": "https://x.test/h" },
            { "id": "rwh_2", "type": "email_bounced", "url": "https://x.test/h" },
        ] }),
    )
    .await;

    let script = format!(
        "{}local hooks, meta = c:webhooks()
         assert.eq(#hooks, 2)
         assert.eq(meta.truncated, false)
         assert.eq(meta.seen, 2)",
        mcp_client(&server.uri())
    );
    run_lua(&script).await.unwrap();
}

/// An empty list still carries its meta, so a caller reading `truncated` on
/// every answer never finds it nil.
#[tokio::test]
async fn test_an_empty_webhook_list_still_carries_meta() {
    let server = MockServer::start().await;
    mount_mcp(&server, "list_webhooks", json!({ "total": 0, "data": [] })).await;
    let script = format!(
        "{}local hooks, meta = c:webhooks()
         assert.eq(#hooks, 0)
         assert.eq(meta.truncated, false)
         assert.eq(meta.cap, 10)",
        mcp_client(&server.uri())
    );
    run_lua(&script).await.unwrap();
}

/// `tostring` on a table sends the vendor "table: 0x…" as an event name, and
/// its refusal arrives seconds later in words nobody can act on.
#[tokio::test]
async fn test_a_url_or_event_that_is_not_a_string_is_refused_here() {
    let server = MockServer::start().await;
    let script = format!(
        "{}local a, e1 = c:create_webhook({{ url = \"https://x.test/h\", event = {{}} }})\n\
         assert.eq(a, nil)\n\
         assert.eq(e1.code, \"config\")\n\
         assert.contains(e1.message, \"string\")\n\
         local b, e2 = c:create_webhook({{ url = {{}}, event = \"email_replied\" }})\n\
         assert.eq(b, nil)\n\
         assert.eq(e2.code, \"config\")",
        mcp_client(&server.uri())
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_a_webhook_needs_a_url_and_an_event() {
    let server = MockServer::start().await;
    let script = format!(
        "{}local a, e1 = c:create_webhook({{ event = \"email_replied\" }})\n\
         assert.eq(a, nil)\n\
         assert.eq(e1.code, \"config\")\n\
         local b, e2 = c:create_webhook({{ url = \"https://x.test/h\" }})\n\
         assert.eq(b, nil)\n\
         assert.eq(e2.code, \"config\")",
        mcp_client(&server.uri())
    );
    run_lua(&script).await.unwrap();
}

/// The vendor refuses inside a 200, in an ordinary-looking result. Read as a
/// payload it becomes a success carrying the word "Error", which is how a
/// caller ends up acting on a call that did nothing.
#[tokio::test]
async fn test_a_refusal_the_tool_made_reads_as_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(header("x-salesforge-key", "k"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "isError": true,
                "content": [{ "type": "text", "text": "Error: Salesforge API error 400: taken" }],
            },
        })))
        .mount(&server)
        .await;

    let script = format!(
        "{}local hook, err = c:create_webhook({{ url = \"https://x.test/h\", event = \"email_replied\" }})\n\
         assert.eq(hook, nil)\n\
         assert.eq(err.code, \"tool\")\n\
         assert.contains(err.message, \"400\")",
        mcp_client(&server.uri())
    );
    run_lua(&script).await.unwrap();
}
