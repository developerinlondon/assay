//! assay.forge against the forge MCP shapes probed live on 2026-09-04,
//! anonymised. What is pinned: the per-product header, the domain name arriving
//! as `sld` and `tld` and never whole, the health report's tri-state where an
//! omitted check is unknown rather than failing, warm-up read off the two halves
//! of the curve, a placement row with no counts reading as no test rather than
//! as zero, Warmforge paging by `totalPages`, the frame being chosen by its
//! JSON-RPC id, and auth and rate limits reading as themselves.

#[path = "../common/mod.rs"]
mod common;

use common::run_lua;
use serde_json::json;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const WS: &str = "wks_xa1";

fn client(product: &str, uri: &str) -> String {
    format!(
        "local f = require(\"assay.forge\")\n\
         local c = f.{product}({{ api_key = \"k\", workspace_id = \"{WS}\", base_url = \"{uri}\" }})\n"
    )
}

/// The tool's own payload arrives as a JSON string nested in the reply's text
/// part, which is why the envelope is parsed twice.
fn reply(payload: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": { "content": [{ "type": "text", "text": payload.to_string() }] },
    }))
}

async fn mount_tool(server: &MockServer, key_header: &str, tool: &str, payload: serde_json::Value) {
    Mock::given(method("POST"))
        .and(path("/"))
        .and(header(key_header, "k"))
        .and(body_string_contains(tool))
        .respond_with(reply(payload))
        .mount(server)
        .await;
}

fn warmforge_row() -> serde_json::Value {
    json!({
        "id": "mbx_xa1", "address": "ada@example.test", "status": "active",
        "provider": "smtp", "warm": false, "warmupEnabled": true,
        "warmupDaysCompleted": 9, "warmupDaysLeft": 5,
        "healthReport": {
            "address": "ada@example.test", "domain": "example.test", "id": "mbx_xa1",
            "heatScore": 82, "warmupDays": 9, "lastCheckedAt": "2026-09-04T01:08:08Z",
            "spf": { "status": "valid", "value": "v=spf1 include:_spf.example.test ~all" },
            "dkim": { "status": "valid", "selector": "google" },
            "mx": { "status": "invalid", "value": "" },
            "blacklists": {
                "detectedCount": 1,
                "checks": [
                    { "id": "clean.example.test", "name": "Clean", "detected": false },
                    { "id": "listed.example.test", "name": "Listed", "detected": true },
                ],
            },
        },
    })
}

/// The vendor answers the name in two halves and never as one string. A reader
/// that looks only for a whole name finds no domains at all.
#[tokio::test]
async fn test_a_primeforge_domain_name_is_sld_and_tld_joined() {
    let server = MockServer::start().await;
    mount_tool(
        &server,
        "X-Primeforge-Key",
        "primeforge_list_domains",
        json!({
            "pagination": { "limit": 10, "offset": 0 },
            "results": [
                { "id": "dom_xa1", "sld": "Example", "tld": "TEST", "status": "active" },
                { "id": "dom_xa2", "status": "active" },
            ],
        }),
    )
    .await;
    run_lua(&format!(
        "{}{}",
        client("primeforge", &server.uri()),
        r#"
        local rows = c:domains()
        assert.eq(#rows, 1)
        assert.eq(rows[1].domain, "example.test")
        assert.eq(rows[1].provider, "primeforge")
        assert.eq(rows[1].provider_ref, "dom_xa1")
        "#
    ))
    .await
    .unwrap();
}

/// `primeforge_list_mailboxes` takes `workspaceId` and nothing else — probed
/// live, it ignores `limit` and `offset` and answers the same ten rows at every
/// offset. So the domain filter is applied to what the vendor gives, and the
/// request must not claim to send one.
#[tokio::test]
async fn test_primeforge_filters_mailboxes_by_domain_without_sending_a_filter() {
    let server = MockServer::start().await;
    mount_tool(
        &server,
        "X-Primeforge-Key",
        "primeforge_list_mailboxes",
        json!({ "results": [
            { "id": "mbx_xa1", "address": "Ada@Example.TEST", "status": "ACTIVE",
              "domainId": "dom_xa1", "password": "hunter2", "appPassword": "abcd efgh" },
            { "id": "mbx_xa2", "address": "bo@other.test", "status": "active",
              "domainId": "dom_xa2" },
        ] }),
    )
    .await;
    run_lua(&format!(
        "{}{}",
        client("primeforge", &server.uri()),
        r#"
        assert.eq(#c:mailboxes(), 2)
        local rows = c:mailboxes("dom_xa1")
        assert.eq(#rows, 1)
        assert.eq(rows[1].address, "ada@example.test")
        assert.eq(rows[1].domain, "example.test")
        assert.eq(rows[1].raw.password, "[redacted]")
        assert.eq(rows[1].raw.appPassword, "[redacted]")
        "#
    ))
    .await
    .unwrap();
    let sent = &server.received_requests().await.unwrap()[0];
    let body = String::from_utf8(sent.body.clone()).unwrap();
    assert!(body.contains(WS), "workspace not sent: {body}");
    assert!(
        !body.contains("limit"),
        "sent a paging argument the tool ignores: {body}"
    );
    assert!(
        !body.contains("domainId"),
        "sent a filter the tool does not accept: {body}"
    );
}

/// Each product authenticates with its own header on the one shared endpoint.
/// Sending Primeforge's header to Warmforge reads as a refused key.
#[tokio::test]
async fn test_each_product_sends_its_own_key_header() {
    let server = MockServer::start().await;
    mount_tool(
        &server,
        "X-Warmforge-Key",
        "warmforge_list_mailboxes",
        json!({ "mailboxes": [warmforge_row()], "page": 1, "pageSize": 50, "totalPages": 1 }),
    )
    .await;
    run_lua(&format!(
        "{}{}",
        client("warmforge", &server.uri()),
        r#"
        local rows = c:mailboxes()
        assert.eq(#rows, 1)
        assert.eq(rows[1].address, "ada@example.test")
        assert.eq(rows[1].provider, "warmforge")
        assert.eq(rows[1].provider_ref, "mbx_xa1")
        "#
    ))
    .await
    .unwrap();
    // A Primeforge client against the same mock never matches the header, so the
    // endpoint answers 404 rather than data.
    run_lua(&format!(
        "{}{}",
        client("primeforge", &server.uri()),
        r#"
        local rows, err = c:mailboxes()
        assert.eq(rows, nil)
        assert.eq(err.code, "http")
        "#
    ))
    .await
    .unwrap();
}

/// A check the report does not mention has not been run. Reading an omitted one
/// as a failure tells an operator a record they published is missing.
#[tokio::test]
async fn test_the_health_report_is_tri_state_and_an_omitted_check_is_unknown() {
    let server = MockServer::start().await;
    mount_tool(
        &server,
        "X-Warmforge-Key",
        "warmforge_list_mailboxes",
        json!({ "mailboxes": [warmforge_row()], "totalPages": 1 }),
    )
    .await;
    run_lua(&format!(
        "{}{}",
        client("warmforge", &server.uri()),
        r#"
        local h = c:health("ADA@example.test")
        assert.eq(h.spf, "valid")
        assert.eq(h.dkim, "valid")
        assert.eq(h.mx, "invalid")
        -- The row carries no dmarc at all. It is not a failing DMARC.
        assert.eq(h.dmarc, "unknown")
        assert.eq(h.heat, 82)
        assert.eq(h.blacklists.detected, 1)
        assert.eq(h.blacklists.lists[1], "listed.example.test")
        "#
    ))
    .await
    .unwrap();
}

/// The vendor reports days done and days left, so the curve's length is their
/// sum rather than a constant this module keeps in step with the vendor's.
#[tokio::test]
async fn test_warmup_day_and_total_come_off_the_two_halves_of_the_curve() {
    let server = MockServer::start().await;
    mount_tool(
        &server,
        "X-Warmforge-Key",
        "warmforge_list_mailboxes",
        json!({ "mailboxes": [warmforge_row()], "totalPages": 1 }),
    )
    .await;
    run_lua(&format!(
        "{}{}",
        client("warmforge", &server.uri()),
        r#"
        local w = c:warmup("ada@example.test")
        assert.eq(w.day, 9)
        assert.eq(w.total_days, 14)
        assert.eq(w.heat, 82)
        assert.eq(w.enabled, true)
        local missing, err = c:warmup("nobody@example.test")
        assert.eq(missing, nil)
        assert.eq(err.code, "not_found")
        "#
    ))
    .await
    .unwrap();
}

/// A heat score outside 0..100, or not a number, is not a reading.
#[tokio::test]
async fn test_a_heat_score_outside_the_scale_is_not_a_reading() {
    run_lua(
        r#"
        local f = require("assay.forge")
        assert.eq(f.heat(82), 82)
        assert.eq(f.heat(140), nil)
        assert.eq(f.heat(-1), nil)
        assert.eq(f.heat("hot"), nil)
        "#,
    )
    .await
    .unwrap();
}

/// The tool answers a row per mailbox whether or not a test has ever run, so a
/// row with no folder counts is a test nobody took and not a placement of zero.
#[tokio::test]
async fn test_a_placement_row_with_no_counts_is_no_test_not_a_zero() {
    let server = MockServer::start().await;
    mount_tool(
        &server,
        "X-Warmforge-Key",
        "warmforge_list_mailboxes",
        json!({ "mailboxes": [warmforge_row()], "totalPages": 1 }),
    )
    .await;
    mount_tool(
        &server,
        "X-Warmforge-Key",
        "warmforge_get_latest_mailbox_placement_results",
        json!({ "results": [
            { "address": "ada@example.test", "mailboxId": "mbx_xa1", "provider": "smtp" },
        ] }),
    )
    .await;
    run_lua(&format!(
        "{}{}",
        client("warmforge", &server.uri()),
        r#"
        local p, err = c:placement("ada@example.test")
        assert.eq(p, nil)
        assert.eq(err, nil)
        local f = require("assay.forge")
        assert.eq(f.placement(80, 15, 5).inbox, 0.8)
        assert.eq(f.placement(0.8, 0.15, 0.05).spam, 0.15)
        assert.eq(f.placement(0, 0, 0), nil)
        "#
    ))
    .await
    .unwrap();
}

/// Warmforge pages for real: `page` and `page_size` are not optional and
/// `totalPages` is the stop condition. A caller that read one page would report
/// a fleet smaller than the one it holds.
#[tokio::test]
async fn test_warmforge_pages_to_total_pages() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("\"page\":1"))
        .respond_with(reply(json!({
            "mailboxes": [{ "id": "mbx_xp1", "address": "one@example.test" }],
            "page": 1, "pageSize": 50, "totalPages": 2,
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("\"page\":2"))
        .respond_with(reply(json!({
            "mailboxes": [{ "id": "mbx_xp2", "address": "two@example.test" }],
            "page": 2, "pageSize": 50, "totalPages": 2,
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        client("warmforge", &server.uri()),
        r#"
        local rows = c:mailboxes()
        assert.eq(#rows, 2)
        assert.eq(rows[2].address, "two@example.test")
        "#
    ))
    .await
    .unwrap();
}

/// A Streamable-HTTP endpoint may answer with a stream, and a stream can carry
/// frames this call did not ask for. Taking the last frame reads a server
/// notification as an answer, so the frame is chosen by its JSON-RPC id.
#[tokio::test]
async fn test_the_reply_frame_is_chosen_by_its_json_rpc_id() {
    let server = MockServer::start().await;
    let answer = json!({ "results": [{ "id": "dom_xa1", "sld": "example", "tld": "test" }] });
    let stream = format!(
        "event: message\ndata: {}\n\nevent: message\ndata: {}\n\n",
        json!({ "jsonrpc": "2.0", "id": 1,
                "result": { "content": [{ "text": answer.to_string() }] } }),
        json!({ "jsonrpc": "2.0", "method": "notifications/progress", "params": {} }),
    );
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(stream, "text/event-stream"))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        client("primeforge", &server.uri()),
        r#"
        local rows = c:domains()
        assert.eq(#rows, 1)
        assert.eq(rows[1].domain, "example.test")
        "#
    ))
    .await
    .unwrap();
}

/// A refused key is not an empty workspace.
#[tokio::test]
async fn test_auth_and_rate_limits_read_as_themselves_not_as_absence() {
    for (status, code) in [
        (401u16, "auth"),
        (403, "auth"),
        (429, "rate_limit"),
        (502, "server"),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let check = format!(
            r#"
            local rows, err = c:domains()
            assert.eq(rows, nil)
            assert.eq(err.code, "{code}")
            assert.eq(err.status, {status})
            assert.contains(tostring(err), "forge: ")
            "#
        );
        run_lua(&format!("{}{check}", client("primeforge", &server.uri())))
            .await
            .unwrap();
    }
}

/// A tool that answers with a JSON-RPC error says so; read as an empty payload
/// it would look like a workspace with nothing in it.
#[tokio::test]
async fn test_a_tool_error_reads_as_a_tool_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0", "id": 1,
            "error": { "code": -32602, "message": "unknown workspace" },
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        "{}{}",
        client("primeforge", &server.uri()),
        r#"
        local rows, err = c:domains()
        assert.eq(rows, nil)
        assert.eq(err.code, "tool")
        assert.contains(err.message, "unknown workspace")
        "#
    ))
    .await
    .unwrap();
}

/// A client with no key, no workspace, or an unknown product is a programming
/// error rather than a vendor answer.
#[tokio::test]
async fn test_a_client_refuses_to_build_without_a_key_or_a_workspace() {
    for args in [r#"{ workspace_id = "wks_xa1" }"#, r#"{ api_key = "k" }"#] {
        let err = run_lua(&format!(
            "local f = require(\"assay.forge\")\nf.primeforge({args})"
        ))
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("required"), "gave {err}");
    }
    run_lua(
        r#"
        local f = require("assay.forge")
        local payload, err = f.mcp("nopeforge", "k", "any_tool")
        assert.eq(payload, nil)
        assert.eq(err.code, "product")
        "#,
    )
    .await
    .unwrap();
}
