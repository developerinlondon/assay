//! assay.reoon against the response shapes probed from the live API on
//! 2026-11-08. What is pinned: a vendor "safe" never becoming VERIFIED, the
//! catch-all override, a refused budget never reaching the vendor, the vendor's
//! own refusal reason surviving into the error, and the key — which travels in
//! the query string — never appearing in an error or in provenance.

#[path = "../common/mod.rs"]
mod common;

use common::run_lua;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sekrit-key-123";

const OPEN_GATE: &str = r#"
local lp = require("assay.lead_provider")
local spent = {}
local gate = lp.gate({
  approve = function(op, cents) spent[#spent + 1] = op .. ":" .. cents; return true end,
  meter = function() end,
})
"#;

const CLOSED_GATE: &str = r#"
local lp = require("assay.lead_provider")
local gate = lp.gate({
  approve = function() return false, "over_cap" end,
  meter = function() end,
})
"#;

fn script(gate: &str, uri: &str, body: &str) -> String {
    format!(
        "{gate}\nlocal reoon = require(\"assay.reoon\")\n\
         local c = reoon.client(gate, {{ api_key = \"{KEY}\", base_url = \"{uri}\" }})\n{body}"
    )
}

async fn mount_verify(server: &MockServer, email: &str, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/api/v1/verify"))
        .and(query_param("email", email))
        .and(query_param("key", KEY))
        .and(query_param("mode", "power"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn passes(gate: &str, uri: &str, body: &str) {
    run_lua(&script(gate, uri, body)).await.unwrap();
}

async fn fails(uri: &str, body: &str) -> mlua::Error {
    run_lua(&script(OPEN_GATE, uri, body)).await.unwrap_err()
}

fn assert_says(err: mlua::Error, want: &str, ctx: &str) {
    let text = err.to_string();
    assert!(text.contains(want), "{ctx} gave {text}, wanted {want}");
    assert!(!text.contains(KEY), "{ctx} leaked the key: {text}");
}

/// "safe" is the vendor's opinion of an address; only delivering to it is
/// proof, so it lands as PROBABLE and the raw verdict rides along.
#[tokio::test]
async fn test_safe_lands_as_probable_with_the_vendor_detail_carried() {
    let server = MockServer::start().await;
    mount_verify(
        &server,
        "jo@shop.example",
        json!({
            "email": "jo@shop.example", "status": "safe", "overall_score": 98,
            "is_catch_all": false, "is_deliverable": true, "is_safe_to_send": true,
            "mx_accepts_mail": true, "can_connect_smtp": true, "is_disposable": false,
            "is_role_account": false
        }),
    )
    .await;
    passes(
        OPEN_GATE,
        &server.uri(),
        r#"
        local r = c:verify_email("jo@shop.example", { cents = 1 })
        assert.eq(r.address, "jo@shop.example")
        assert.eq(r.verification_status, "PROBABLE")
        assert.eq(r.vendor_result, "safe")
        assert.eq(r.confidence, 98)
        assert.eq(r.is_safe_to_send, true)
        assert.eq(r.provenance.provider, "reoon")
        assert.eq(r.provenance.retrieved_from:find("key=", 1, true), nil)
        assert.contains(r.provenance.retrieved_from, "/api/v1/verify")
        assert.eq(spent[1], "verify_email:1")
        "#,
    )
    .await;
}

/// A host that accepts anything says nothing about this mailbox, whatever the
/// vendor's headline verdict was.
#[tokio::test]
async fn test_a_catch_all_flag_overrides_the_headline_status() {
    let server = MockServer::start().await;
    mount_verify(
        &server,
        "a@accepts-all.example",
        json!({ "status": "safe", "is_catch_all": true }),
    )
    .await;
    mount_verify(
        &server,
        "b@accepts-all.example",
        json!({ "status": "catch_all", "is_catch_all": true }),
    )
    .await;
    passes(
        OPEN_GATE,
        &server.uri(),
        r#"
        assert.eq(c:verify_email("a@accepts-all.example").verification_status, "CATCH_ALL")
        assert.eq(c:verify_email("b@accepts-all.example").verification_status, "CATCH_ALL")
        "#,
    )
    .await;
}

#[tokio::test]
async fn test_every_vendor_status_buckets_into_the_contract_vocabulary() {
    run_lua(
        r#"
        local reoon = require("assay.reoon")
        local want = {
          safe = "PROBABLE", catch_all = "CATCH_ALL",
          invalid = "INVALID", disabled = "INVALID", spamtrap = "INVALID", disposable = "INVALID",
          role_account = "UNKNOWN", inbox_full = "UNKNOWN", unknown = "UNKNOWN",
          something_new = "UNKNOWN",
        }
        for status, bucket in pairs(want) do
          assert.eq(reoon.verdict(status, false), bucket)
          assert.eq(reoon.verdict(status, false) ~= "VERIFIED", true)
        end
        "#,
    )
    .await
    .unwrap();
}

/// A refused budget answers (nil, reason) and the vendor is never called.
#[tokio::test]
async fn test_a_declined_budget_never_reaches_the_vendor() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/verify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "safe" })))
        .expect(0)
        .mount(&server)
        .await;
    passes(
        CLOSED_GATE,
        &server.uri(),
        r#"
        local r, reason = c:verify_email("a@b.example")
        assert.eq(r, nil)
        assert.eq(reason, "over_cap")
        "#,
    )
    .await;
}

/// Reoon answers a refusal with `{status:"error", reason}`. The reason is what
/// an operator needs — "out of credits" and "bad key" are different jobs.
#[tokio::test]
async fn test_refusals_carry_the_vendor_reason_and_never_the_key() {
    for (status, reason, want) in [
        (401u16, "Invalid API key.", "rejected the key"),
        (429, "Too many requests.", "rate limited"),
        (402, "Not enough credits.", "HTTP 402: Not enough credits."),
        (200, "Not enough credits.", "error: Not enough credits."),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/verify"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_json(json!({ "status": "error", "reason": reason })),
            )
            .mount(&server)
            .await;
        let err = fails(&server.uri(), r#"c:verify_email("a@b.example")"#).await;
        assert_says(err, want, &format!("HTTP {status}"));
    }
}

/// A transport failure names the URL it was fetching, and that URL carries the
/// key. Nothing may reach a log with it.
#[tokio::test]
async fn test_a_transport_error_is_scrubbed_of_the_key() {
    let err = fails("http://127.0.0.1:1", r#"c:verify_email("a@b.example")"#).await;
    assert_says(err, "reoon: verify failed", "unreachable host");
}

/// A key full of Lua pattern characters, echoed back raw in a vendor body: the
/// scrub must escape it rather than read it as a pattern.
#[tokio::test]
async fn test_a_key_echoed_raw_is_scrubbed_whatever_it_contains() {
    let odd = "a.b%c-d+e(f)";
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/verify"))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_json(json!({ "status": "error", "reason": format!("bad key {odd}") })),
        )
        .mount(&server)
        .await;
    let text = run_lua(&format!(
        "{OPEN_GATE}\nlocal reoon = require(\"assay.reoon\")\n\
         local c = reoon.client(gate, {{ api_key = \"{odd}\", base_url = \"{}\" }})\n\
         c:verify_email(\"a@b.example\")",
        server.uri()
    ))
    .await
    .unwrap_err()
    .to_string();
    assert!(text.contains("HTTP 500: bad key <redacted>"), "gave {text}");
    assert!(!text.contains(odd), "leaked the key: {text}");
}

const READS_THE_BALANCE: &str = r#"
local b = c:balance()
assert.eq(b.api_status, "active")
assert.eq(b.daily, 3)
assert.eq(b.instant, 41)
"#;

/// The balance read is free, so it is what a connection test should call.
#[tokio::test]
async fn test_balance_reads_remaining_credits() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/check-account-balance/"))
        .and(query_param("key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "api_status": "active", "remaining_daily_credits": 3,
            "remaining_instant_credits": 41, "status": "success"
        })))
        .mount(&server)
        .await;
    passes(CLOSED_GATE, &server.uri(), READS_THE_BALANCE).await;
}

#[tokio::test]
async fn test_refuses_to_build_without_a_gate_or_a_key() {
    for (args, want) in [
        (r#"nil, { api_key = "k" }"#, "gate is required"),
        (r#"gate, { api_key = "" }"#, "api key required"),
    ] {
        let err = run_lua(&format!(
            "{OPEN_GATE}\nlocal reoon = require(\"assay.reoon\")\nreoon.client({args})"
        ))
        .await
        .unwrap_err();
        assert_says(err, want, args);
    }
}
