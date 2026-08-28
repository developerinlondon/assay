//! assay.mails_so against the response shape probed from the live API on
//! 2026-08-28. What is pinned: the header name, verdicts never laundering into
//! VERIFIED, the catch-all override, a refused budget never reaching the
//! vendor, and auth/rate-limit reading as themselves rather than as absence.

#[path = "../common/mod.rs"]
mod common;

use common::run_lua;
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
        "{gate}\nlocal ms = require(\"assay.mails_so\")\n\
         local c = ms.client(gate, {{ api_key = \"k\", base_url = \"{uri}\" }})\n{body}"
    )
}

fn validate_response(data: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "data": data, "error": null }))
}

async fn mount_validate(server: &MockServer, email: &str, data: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/v1/validate"))
        .and(header("x-mails-api-key", "k"))
        .and(query_param("email", email))
        .respond_with(validate_response(data))
        .mount(server)
        .await;
}

fn assert_says(err: mlua::Error, want: &str, ctx: &str) {
    let text = err.to_string();
    assert!(text.contains(want), "{ctx} gave {text}, wanted {want}");
}

/// "deliverable" is the vendor's opinion; it lands as PROBABLE, never VERIFIED.
#[tokio::test]
async fn test_deliverable_lands_as_probable_with_the_vendor_detail_carried() {
    let server = MockServer::start().await;
    mount_validate(
        &server,
        "jc@cheaney.co.uk",
        json!({
            "email": "jc@cheaney.co.uk", "result": "deliverable", "reason": "accepted_email",
            "score": 92, "mx_record": "mx.cheaney.co.uk", "provider": "microsoft",
            "isv_nocatchall": true, "is_disposable": false, "is_free": false,
            "did_you_mean": null
        }),
    )
    .await;
    run_lua(&script(
        OPEN_GATE,
        &server.uri(),
        r#"
        local r = c:verify_email("jc@cheaney.co.uk")
        assert.eq(r.address, "jc@cheaney.co.uk")
        assert.eq(r.verification_status, "PROBABLE")
        assert.eq(r.vendor_result, "deliverable")
        assert.eq(r.confidence, 92)
        assert.eq(r.mx_record, "mx.cheaney.co.uk")
        assert.eq(r.provenance.provider, "mails_so")
        assert.not_nil(r.provenance.retrieved_at)
        assert.eq(spent[1], "verify_email:0")
        "#,
    ))
    .await
    .unwrap();
}

/// A domain that accepts anything is CATCH_ALL whatever the vendor concluded,
/// and CATCH_ALL never schedules.
#[tokio::test]
async fn test_a_catch_all_domain_overrides_a_deliverable_verdict() {
    let server = MockServer::start().await;
    mount_validate(
        &server,
        "a@accepts-all.example",
        json!({ "result": "deliverable", "isv_nocatchall": false, "score": 80 }),
    )
    .await;
    run_lua(&script(
        OPEN_GATE,
        &server.uri(),
        r#"
        local r = c:verify_email("a@accepts-all.example")
        assert.eq(r.verification_status, "CATCH_ALL")
        assert.eq(r.vendor_result, "deliverable")
        "#,
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn test_undeliverable_is_invalid_and_risky_is_unknown() {
    let server = MockServer::start().await;
    mount_validate(
        &server,
        "gone@x.example",
        json!({ "result": "undeliverable", "reason": "rejected_email", "isv_nocatchall": true }),
    )
    .await;
    mount_validate(&server, "meh@x.example", json!({ "result": "risky", "isv_nocatchall": true }))
        .await;
    run_lua(&script(
        OPEN_GATE,
        &server.uri(),
        r#"
        local dead = c:verify_email("gone@x.example")
        assert.eq(dead.verification_status, "INVALID")
        assert.eq(dead.reason, "rejected_email")
        assert.eq(c:verify_email("meh@x.example").verification_status, "UNKNOWN")
        "#,
    ))
    .await
    .unwrap();
}

/// A refused budget answers (nil, reason) and the vendor is never called.
#[tokio::test]
async fn test_a_declined_budget_never_reaches_the_vendor() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/validate"))
        .respond_with(validate_response(json!({ "result": "deliverable" })))
        .expect(0)
        .mount(&server)
        .await;
    run_lua(&script(
        CLOSED_GATE,
        &server.uri(),
        r#"
        local r, reason = c:verify_email("a@b.example")
        assert.eq(r, nil)
        assert.eq(reason, "over_cap")
        "#,
    ))
    .await
    .unwrap();
}

/// A rejected key and an exhausted quota must not read as a verdict.
#[tokio::test]
async fn test_auth_and_rate_limit_read_as_themselves() {
    for (status, want) in [(401u16, "rejected the key"), (429, "rate limited")] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/validate"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let err = run_lua(&script(OPEN_GATE, &server.uri(), r#"c:verify_email("a@b.example")"#))
            .await
            .unwrap_err();
        assert_says(err, want, &format!("HTTP {status}"));
    }
}

/// The API reports its own failures in-band; a non-null error is an error.
#[tokio::test]
async fn test_an_in_band_api_error_is_an_error_not_a_verdict() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": null, "error": "invalid email parameter"
        })))
        .mount(&server)
        .await;
    let err = run_lua(&script(OPEN_GATE, &server.uri(), r#"c:verify_email("nope")"#))
        .await
        .unwrap_err();
    assert_says(err, "invalid email parameter", "in-band error");
}

/// A client buildable without a gate or key makes an unmetered or unauthed
/// call reachable by omission.
#[tokio::test]
async fn test_refuses_to_build_without_a_gate_or_a_key() {
    for (args, want) in [
        (r#"nil, { api_key = "k" }"#, "gate is required"),
        (r#"gate, { api_key = "" }"#, "api key required"),
    ] {
        let err = run_lua(&format!(
            "{OPEN_GATE}\nlocal ms = require(\"assay.mails_so\")\nms.client({args})"
        ))
        .await
        .unwrap_err();
        assert_says(err, want, args);
    }
}
