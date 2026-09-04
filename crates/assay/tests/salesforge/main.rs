//! assay.salesforge against the shapes probed live on 2026-09-04, anonymised.
//! What is pinned: the public key riding bare in `Authorization` with no Bearer
//! prefix, an empty list arriving as a JSON object rather than an array, paging
//! by limit and offset, the public API carrying no warm-up state while the
//! internal one does, a failed sign-in reading as a typed sign_in error that
//! leaves the public API working, and 401, 402 and 429 reading as themselves.

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
    for (status, code) in [
        (401u16, "auth"),
        (403, "auth"),
        (402, "plan"),
        (429, "rate_limit"),
        (500, "server"),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/public/v2/workspaces/{WS}/mailboxes")))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let check = format!(
            r#"
            local rows, err = c:mailboxes()
            assert.eq(rows, nil)
            assert.eq(err.code, "{code}")
            assert.eq(err.status, {status})
            "#
        );
        run_lua(&format!("{}{check}", client(&server.uri(), "")))
            .await
            .unwrap();
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
