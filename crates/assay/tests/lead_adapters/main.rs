//! ContactOut and BetterContact against recorded response shapes. The live
//! smoke test waits on keys; what can be pinned without one is pinned here —
//! the auth header each vendor wants, the gate being unbypassable, an async
//! run only counting as finished when it says so, and vendor claims never
//! laundering into VERIFIED.

#[path = "../common/mod.rs"]
mod common;

use common::run_lua;
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Approves everything and records what it was asked to spend.
const OPEN_GATE: &str = r#"
local lp = require("assay.lead_provider")
local spent = {}
local gate = lp.gate({
  approve = function(op, cents) spent[#spent + 1] = op .. ":" .. cents; return true end,
  meter = function() end,
})
"#;

/// Refuses everything, so a test can prove the vendor is never called.
const CLOSED_GATE: &str = r#"
local lp = require("assay.lead_provider")
local gate = lp.gate({
  approve = function() return false, "over_cap" end,
  meter = function() end,
})
"#;

fn script(gate: &str, module: &str, alias: &str, ctor: &str, uri: &str, body: &str) -> String {
    format!(
        "{gate}\nlocal {alias} = require(\"assay.{module}\")\n\
         local c = {alias}.client(gate, {{ {ctor}, base_url = \"{uri}\" }})\n{body}"
    )
}

async fn contactout(uri: &str, body: &str) -> Result<(), mlua::Error> {
    run_lua(&script(OPEN_GATE, "contactout", "co", r#"token = "k""#, uri, body)).await
}

async fn bettercontact(gate: &str, uri: &str, body: &str) -> Result<(), mlua::Error> {
    run_lua(&script(gate, "bettercontact", "bc", r#"api_key = "bc-key""#, uri, body)).await
}

async fn server_returning(m: &str, p: &str, status: u16) -> MockServer {
    let server = MockServer::start().await;
    let mock = if m == "POST" {
        Mock::given(method("POST")).and(path(p.to_string()))
    } else {
        Mock::given(method("GET")).and(path(p.to_string()))
    };
    mock.respond_with(ResponseTemplate::new(status)).mount(&server).await;
    server
}

fn assert_says(err: mlua::Error, want: &str, ctx: &str) {
    let text = err.to_string();
    assert!(text.contains(want), "{ctx} gave {text}, wanted {want}");
}

// ---------------------------------------------------------------------------
// ContactOut
// ---------------------------------------------------------------------------

/// The header is the bare name `token` — not `Authorization`, no `Bearer`.
/// Getting it wrong reads as an invalid key rather than a malformed request.
#[tokio::test]
async fn test_contactout_sends_the_bare_token_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/linkedin/enrich"))
        .and(header("token", "k"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status_code": 200,
            "profile": {
                "url": "https://www.linkedin.com/in/jchurch",
                "full_name": "Jonathan Church",
                "headline": "Managing Director",
                "work_email": ["jonathan.church@cheaney.co.uk"],
                "email": ["jonathan.church@cheaney.co.uk", "jc@personal.example"]
            }
        })))
        .mount(&server)
        .await;
    contactout(
        &server.uri(),
        r#"
        local p = c:enrich_linkedin("https://www.linkedin.com/in/jchurch")
        assert.eq(p.full_name, "Jonathan Church")
        assert.eq(p.first_name, "Jonathan")
        assert.eq(p.last_name, "Church")
        assert.eq(p.title, "Managing Director")
        assert.eq(#p.emails, 2)
        assert.eq(p.emails[1].address, "jonathan.church@cheaney.co.uk")
        assert.eq(p.emails[1].verification_status, "UNKNOWN")
        assert.eq(p.provenance.provider, "contactout")
        assert.not_nil(p.provenance.retrieved_at)
        assert.eq(spent[1], "find_person:0")
        "#,
    )
    .await
    .unwrap();
}

/// Work email leads even when the general list repeats it: outbound writes to
/// a desk, not to someone's personal address.
#[tokio::test]
async fn test_contactout_puts_work_email_first_without_duplicating_it() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/people/person"))
        .and(query_param("email", "jc@cheaney.co.uk"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status_code": 200,
            "profile": {
                "email": ["jc@cheaney.co.uk"],
                "work_email": ["jc@cheaney.co.uk"],
                "linkedin": "https://www.linkedin.com/in/jc"
            }
        })))
        .mount(&server)
        .await;
    contactout(
        &server.uri(),
        r#"
        local p = c:profile_by_email("jc@cheaney.co.uk")
        assert.eq(#p.emails, 1)
        assert.eq(p.linkedin, "https://www.linkedin.com/in/jc")
        "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_contactout_reports_an_unknown_person_as_nil_not_an_error() {
    let server = server_returning("GET", "/v1/linkedin/enrich", 404).await;
    contactout(
        &server.uri(),
        r#"assert.eq(c:enrich_linkedin("https://www.linkedin.com/in/nobody"), nil)"#,
    )
    .await
    .unwrap();
}

/// A rejected key and an exhausted quota must not read as "no such person".
#[tokio::test]
async fn test_contactout_distinguishes_auth_and_rate_limit_from_absence() {
    for (status, want) in [(401u16, "rejected the token"), (429, "rate limited")] {
        let server = server_returning("GET", "/v1/linkedin/enrich", status).await;
        let err = contactout(&server.uri(), r#"c:enrich_linkedin("https://x/in/y")"#)
            .await
            .unwrap_err();
        assert_says(err, want, &format!("HTTP {status}"));
    }
}

/// A client buildable without a gate makes an unmetered paid call reachable
/// by omission.
#[tokio::test]
async fn test_contactout_refuses_to_build_without_a_gate_or_a_token() {
    for (args, want) in [
        (r#"nil, { token = "k" }"#, "gate is required"),
        (r#"{}, { token = "k" }"#, "gate is required"),
        (r#"gate, { token = "" }"#, "token required"),
    ] {
        let err = run_lua(&format!(
            "{OPEN_GATE}\nlocal co = require(\"assay.contactout\")\nco.client({args})"
        ))
        .await
        .unwrap_err();
        assert_says(err, want, args);
    }
}

// ---------------------------------------------------------------------------
// BetterContact
// ---------------------------------------------------------------------------

const RUN_ID: &str = "fefbc2203558eb3adcea";

async fn mount_submit(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/async"))
        .and(header("x-api-key", "bc-key"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "success": true, "id": RUN_ID, "message": "Processing..."
        })))
        .mount(server)
        .await;
}

async fn mount_result(server: &MockServer, status: u16, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!("/async/{RUN_ID}")))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

async fn run_server(status: u16, body: serde_json::Value) -> MockServer {
    let server = MockServer::start().await;
    mount_submit(&server).await;
    mount_result(&server, status, body).await;
    server
}

#[tokio::test]
async fn test_bettercontact_submits_then_reads_the_terminated_run() {
    let server = run_server(
        200,
        json!({
            "id": RUN_ID, "status": "terminated",
            "credits_consumed": 1, "credits_left": 331,
            "summary": { "total": 1, "valid": 1 },
            "data": [{
                "contact_first_name": "Jonathan",
                "contact_last_name": "Church",
                "contact_job_title": "Managing Director",
                "contact_email_address": "jonathan.church@cheaney.co.uk",
                "contact_email_status": "valid",
                "company_name": "Joseph Cheaney & Sons",
                "company_domain": "cheaney.co.uk",
                "contact_location_country": "United Kingdom"
            }]
        }),
    )
    .await;
    bettercontact(
        OPEN_GATE,
        &server.uri(),
        r#"
        local p = c:find_person({ first_name = "Jonathan", last_name = "Church",
          domain = "cheaney.co.uk" }, { poll_ms = 1, attempts = 3 })
        assert.eq(p.full_name, "Jonathan Church")
        assert.eq(p.company, "Joseph Cheaney & Sons")
        assert.eq(p.domain, "cheaney.co.uk")
        assert.eq(#p.emails, 1)
        assert.eq(p.emails[1].address, "jonathan.church@cheaney.co.uk")
        assert.eq(p.provenance.provider, "bettercontact")
        assert.eq(spent[1], "find_person:0")
        "#,
    )
    .await
    .unwrap();
}

/// `valid` from a vendor is an assertion, not a delivery, so it stops at
/// PROBABLE. Only a real send earns VERIFIED (NEP-0007 §2).
#[tokio::test]
async fn test_bettercontact_never_launders_a_vendor_claim_into_verified() {
    for (vendor, want) in [
        ("valid", "PROBABLE"),
        ("catch_all", "CATCH_ALL"),
        ("undeliverable", "INVALID"),
        ("not_found", "UNKNOWN"),
        ("something_new", "UNKNOWN"),
    ] {
        let server = run_server(
            200,
            json!({
                "id": RUN_ID, "status": "terminated",
                "data": [{ "contact_email_address": "a@b.com", "contact_email_status": vendor }]
            }),
        )
        .await;
        bettercontact(
            OPEN_GATE,
            &server.uri(),
            &format!(
                r#"
                local emails = c:resolve_email({{ first_name = "A", last_name = "B" }},
                  {{ poll_ms = 1, attempts = 2 }})
                assert.eq(emails[1].verification_status, "{want}")
                "#
            ),
        )
        .await
        .unwrap();
    }
}

/// A 202 while processing carries no `data`, so branching on the HTTP code
/// would read an in-flight run as an empty result.
#[tokio::test]
async fn test_bettercontact_treats_only_terminated_as_finished() {
    let server = run_server(
        202,
        json!({ "id": RUN_ID, "status": "processing", "message": "Processing..." }),
    )
    .await;
    bettercontact(
        OPEN_GATE,
        &server.uri(),
        &format!(
            r#"
            local run = c:result("{RUN_ID}")
            assert.eq(run.status, "processing")
            assert.eq(run.terminated, false)
            assert.eq(#run.people, 0)

            local p, reason = c:find_person({{ first_name = "A", last_name = "B" }},
              {{ poll_ms = 1, attempts = 2 }})
            assert.eq(p, nil)
            assert.eq(reason, "not_terminated")
            "#
        ),
    )
    .await
    .unwrap();
}

/// A declined budget must stop the submission ever reaching the vendor —
/// the difference between a gate and an audit log.
#[tokio::test]
async fn test_bettercontact_declined_budget_never_reaches_the_vendor() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/async"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    bettercontact(
        CLOSED_GATE,
        &server.uri(),
        r#"
        local p, reason = c:find_person({ first_name = "A", last_name = "B" })
        assert.eq(p, nil)
        assert.eq(reason, "over_cap")
        "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_bettercontact_distinguishes_auth_credits_and_rate_limit() {
    for (status, want) in [
        (401u16, "rejected the key"),
        (402, "out of credits"),
        (429, "rate limited"),
    ] {
        let server = server_returning("POST", "/async", status).await;
        let err = bettercontact(
            OPEN_GATE,
            &server.uri(),
            r#"c:submit({ { first_name = "A", last_name = "B" } })"#,
        )
        .await
        .unwrap_err();
        assert_says(err, want, &format!("HTTP {status}"));
    }
}

#[tokio::test]
async fn test_bettercontact_refuses_to_build_or_submit_without_the_essentials() {
    for (stmt, want) in [
        (r#"bc.client(nil, { api_key = "k" })"#, "gate is required"),
        (r#"bc.client(gate, { api_key = "" })"#, "api_key required"),
        (
            r#"bc.client(gate, { api_key = "k", base_url = "http://x" }):submit({})"#,
            "at least one person",
        ),
    ] {
        let err = run_lua(&format!(
            "{OPEN_GATE}\nlocal bc = require(\"assay.bettercontact\")\n{stmt}"
        ))
        .await
        .unwrap_err();
        assert_says(err, want, stmt);
    }
}
