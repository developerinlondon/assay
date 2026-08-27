mod common;

use common::run_lua;
use serde_json::json;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_syntax_gate() {
    run_lua(
        r#"
        local ev = require("assay.email_verify")
        assert.eq(ev.check_syntax("jonathan.church@cheaney.co.uk"), true)
        for _, bad in ipairs({
            "", "no-at-sign", "two@@ats.com", "a..b@x.com", ".lead@x.com",
            "x@-bad.com", "x@nodot",
        }) do
            local ok = ev.check_syntax(bad)
            assert.eq(ok, false)
        end
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_candidates_common_shapes_first() {
    run_lua(
        r#"
        local ev = require("assay.email_verify")
        local c = ev.candidates(" Jonathan ", "Church", "www.Cheaney.co.uk")
        assert.eq(c[1], "jonathan.church@cheaney.co.uk")
        assert.eq(c[2], "jonathanchurch@cheaney.co.uk")
        assert.contains(table.concat(c, " "), "jchurch@cheaney.co.uk")
        assert.eq(#ev.candidates("", "x", "y.com"), 0)
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_verify_mx_present_is_unknown_never_more() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/dns-query"))
        .and(query_param("type", "MX"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Answer": [
                { "data": "20 backup.mail.example." },
                { "data": "10 primary.mail.example." }
            ]
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        r#"
        local ev = require("assay.email_verify")
        local c = ev.client({{ doh_url = "{}/dns-query" }})
        local v = c:verify("someone@cheaney.co.uk")
        assert.eq(v.status, "UNKNOWN")
        assert.eq(v.method, "dns:mx")
        assert.eq(v.mx[1].host, "primary.mail.example")
        assert.not_nil(v.checked_at)
    "#,
        server.uri()
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn test_verify_dead_domain_is_invalid() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/dns-query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "Answer": [] })))
        .mount(&server)
        .await;
    run_lua(&format!(
        r#"
        local ev = require("assay.email_verify")
        local c = ev.client({{ doh_url = "{}/dns-query" }})
        local v = c:verify("x@dead.example.com")
        assert.eq(v.status, "INVALID")
        assert.eq(v.method, "dns:no_mx_no_a")
    "#,
        server.uri()
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn test_verify_a_fallback_is_marked_weak() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/dns-query"))
        .and(query_param("type", "MX"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "Answer": [] })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/dns-query"))
        .and(query_param("type", "A"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Answer": [ { "data": "192.0.2.10" } ]
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        r#"
        local ev = require("assay.email_verify")
        local c = ev.client({{ doh_url = "{}/dns-query" }})
        local v = c:verify("x@mxless.example.com")
        assert.eq(v.status, "UNKNOWN")
        assert.eq(v.method, "dns:a_fallback")
    "#,
        server.uri()
    ))
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// smtp_probe: the SMTP evidence rung, against scripted mail servers
// ---------------------------------------------------------------------------

/// Serve one connection: the greeting first, then one scripted reply per
/// command line received. A reply may embed `\r\n` to exercise continuations.
async fn scripted_smtp(script: Vec<&'static str>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut session = BufReader::new(stream);
        let mut replies = script.into_iter();
        let greeting = format!("{}\r\n", replies.next().unwrap());
        session
            .get_mut()
            .write_all(greeting.as_bytes())
            .await
            .unwrap();
        for reply in replies {
            let mut command = String::new();
            if session.read_line(&mut command).await.unwrap_or(0) == 0 {
                return;
            }
            if session
                .get_mut()
                .write_all(format!("{reply}\r\n").as_bytes())
                .await
                .is_err()
            {
                return;
            }
        }
    });
    port
}

/// The probe result as owned Rust values. Reading the fields out here keeps
/// the assertions from touching a Lua table whose VM has already been dropped.
struct ProbeResult {
    host_exists: bool,
    catch_all: bool,
    deliverable: bool,
    full_inbox: bool,
    blocked: bool,
    greylisted: bool,
    code: Option<u16>,
    reason: String,
    stage: String,
    mx_host: Option<String>,
}

async fn probe_opts(port: u16, extra: &str) -> ProbeResult {
    let vm = common::create_vm();
    let script = format!(
        r#"
        return smtp_probe.check({{
          email = "jane.doe@example.test", mx = {{ "127.0.0.1" }},
          from = "probe@sender.test", port = {port}, greylist_delay_ms = 0,
          connect_timeout_ms = 2000, op_timeout_ms = 2000, {extra}
        }})
        "#
    );
    let t = vm.load(script).eval_async::<mlua::Table>().await.unwrap();
    ProbeResult {
        host_exists: t.get("host_exists").unwrap(),
        catch_all: t.get("catch_all").unwrap(),
        deliverable: t.get("deliverable").unwrap(),
        full_inbox: t.get("full_inbox").unwrap(),
        blocked: t.get("blocked").unwrap(),
        greylisted: t.get("greylisted").unwrap(),
        code: t.get("code").unwrap(),
        reason: t.get("reason").unwrap(),
        stage: t.get("stage").unwrap(),
        mx_host: t.get("mx_host").unwrap(),
    }
}

async fn probe(port: u16) -> ProbeResult {
    probe_opts(port, "").await
}

/// The reply a host gives to the unowned catch-all address, when the test
/// wants that host to be an ordinary one rather than a catch-all.
const REJECTS_RANDOM: &str = "550 5.1.1 <random>: Recipient address rejected: User unknown";

#[tokio::test]
async fn test_probe_accepted_recipient_is_deliverable() {
    let port = scripted_smtp(vec![
        "220 mx.example.test ESMTP",
        "250-mx.example.test\r\n250-PIPELINING\r\n250 SIZE 10240000",
        "250 2.1.0 Ok",
        REJECTS_RANDOM,
        "250 2.1.5 Ok",
        "221 2.0.0 Bye",
    ])
    .await;
    let r = probe(port).await;
    assert!(r.host_exists);
    assert!(r.deliverable);
    assert!(!r.catch_all);
    assert_eq!(r.reason.as_str(), "accepted");
    assert_eq!(r.stage.as_str(), "rcpt_target");
    assert_eq!(r.code, Some(250));
    assert_eq!(r.mx_host.as_deref(), Some("127.0.0.1"));
}

#[tokio::test]
async fn test_probe_host_accepting_an_unowned_address_is_catch_all() {
    let port = scripted_smtp(vec![
        "220 mx.example.test ESMTP",
        "250 mx.example.test",
        "250 Ok",
        "250 Ok",
    ])
    .await;
    let r = probe(port).await;
    assert!(r.catch_all);
    assert!(!r.deliverable);
    assert_eq!(r.reason.as_str(), "catch_all");
    assert_eq!(r.stage.as_str(), "rcpt_random");
}

#[tokio::test]
async fn test_probe_rejected_target_is_an_absent_mailbox_not_an_absent_host() {
    let port = scripted_smtp(vec![
        "220 mx.example.test ESMTP",
        "250 mx.example.test",
        "250 Ok",
        REJECTS_RANDOM,
        "550 5.1.1 <jane.doe@example.test>: Recipient address rejected",
        "221 Bye",
    ])
    .await;
    let r = probe(port).await;
    assert!(r.host_exists);
    assert!(!r.deliverable);
    assert_eq!(r.reason.as_str(), "no_mailbox");
    assert_eq!(r.code, Some(550));
}

#[tokio::test]
async fn test_probe_soft_refusal_is_greylisted_not_missing() {
    let port = scripted_smtp(vec![
        "220 mx.example.test ESMTP",
        "250 mx.example.test",
        "250 Ok",
        REJECTS_RANDOM,
        "451 4.7.1 Greylisted, try again later",
        "221 Bye",
    ])
    .await;
    let r = probe(port).await;
    assert!(r.greylisted);
    assert!(!r.deliverable);
    assert_eq!(r.reason.as_str(), "exceeded_limits");
}

#[tokio::test]
async fn test_probe_full_mailbox_is_kept_apart_from_a_missing_one() {
    let port = scripted_smtp(vec![
        "220 mx.example.test ESMTP",
        "250 mx.example.test",
        "250 Ok",
        REJECTS_RANDOM,
        "452 4.2.2 Mailbox is over quota",
        "221 Bye",
    ])
    .await;
    let r = probe(port).await;
    assert!(r.full_inbox);
    assert_eq!(r.reason.as_str(), "full_inbox");
}

#[tokio::test]
async fn test_probe_reputation_block_is_never_a_verdict_on_the_mailbox() {
    let port = scripted_smtp(vec![
        "220 mx.example.test ESMTP",
        "250 mx.example.test",
        "554 5.7.1 Service unavailable; client blocked using Spamhaus",
    ])
    .await;
    let r = probe(port).await;
    assert!(r.blocked);
    assert!(!r.deliverable);
    assert_eq!(r.stage.as_str(), "mail_from");
    assert_eq!(r.reason.as_str(), "blocked");
}

#[tokio::test]
async fn test_probe_falls_back_to_helo_when_ehlo_is_refused() {
    let port = scripted_smtp(vec![
        "220 mx.example.test SMTP",
        "500 5.5.1 Command not recognized",
        "250 mx.example.test",
        "250 Ok",
        REJECTS_RANDOM,
        "250 Ok",
        "221 Bye",
    ])
    .await;
    let r = probe(port).await;
    assert!(r.deliverable);
    assert_eq!(r.stage.as_str(), "rcpt_target");
}

#[tokio::test]
async fn test_probe_can_skip_the_catch_all_check() {
    let port = scripted_smtp(vec![
        "220 mx.example.test ESMTP",
        "250 mx.example.test",
        "250 Ok",
        "250 Ok",
        "221 Bye",
    ])
    .await;
    let r = probe_opts(port, "catch_all = false,").await;
    assert!(r.deliverable);
    assert!(!r.catch_all);
    assert_eq!(r.stage.as_str(), "rcpt_target");
}

#[tokio::test]
async fn test_probe_nothing_listening_never_claims_the_host_exists() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let r = probe(port).await;
    assert!(!r.host_exists);
    assert_eq!(r.reason.as_str(), "unreachable");
    assert_eq!(r.stage.as_str(), "connect");
}

#[tokio::test]
async fn test_probe_silent_host_times_out_rather_than_hanging() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(stream);
    });
    let r = probe_opts(port, "").await;
    assert!(!r.host_exists);
    assert_eq!(r.reason.as_str(), "unreachable");
}

#[tokio::test]
async fn test_probe_refuses_arguments_it_cannot_work_without() {
    let vm = common::create_vm();
    for (args, want) in [
        (
            r#"{ email = "a@b.test", from = "p@s.test" }"#,
            "mx list is required",
        ),
        (
            r#"{ email = "a@b.test", from = "p@s.test", mx = {} }"#,
            "mx list is empty",
        ),
        (
            r#"{ email = "nope", from = "p@s.test", mx = {"h"} }"#,
            "must contain '@'",
        ),
        (
            r#"{ email = "a@b.test", from = "nope", mx = {"h"} }"#,
            "must be a full address",
        ),
    ] {
        let err = vm
            .load(format!("return smtp_probe.check({args})"))
            .eval_async::<mlua::Value>()
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains(want), "{err} should mention {want}");
    }
}

#[tokio::test]
async fn test_list_signals_are_flags_and_never_a_status_on_their_own() {
    run_lua(
        r#"
        local ev = require("assay.email_verify")
        assert.eq(ev.is_disposable("MAILINATOR.com"), true)
        assert.eq(ev.is_disposable("www.yopmail.com"), true)
        assert.eq(ev.is_disposable("cheaney.co.uk"), false)
        assert.eq(ev.is_role("info@cheaney.co.uk"), true)
        assert.eq(ev.is_role("no-reply@cheaney.co.uk"), true)
        assert.eq(ev.is_role("jonathan.church@cheaney.co.uk"), false)
        assert.eq(ev.suggest("jane@gmial.com"), "jane@gmail.com")
        assert.eq(ev.suggest("jane@hotmial.com"), "jane@hotmail.com")
        assert.eq(ev.suggest("jane@gmail.com"), nil)
        assert.eq(ev.suggest("jonathan.church@cheaney.co.uk"), nil)
    "#,
    )
    .await
    .unwrap();
}

/// A probe needs a real envelope sender: receiving servers judge it, and a
/// bogus one costs the probing IP reputation that outlives the lookup.
#[tokio::test]
async fn test_probe_requires_an_envelope_sender() {
    let err = run_lua(
        r#"
        local ev = require("assay.email_verify")
        ev.client():probe("jane@example.com", {})
    "#,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("opts.from"), "{err}");
}

#[tokio::test]
async fn test_probe_stops_at_dns_when_the_domain_cannot_receive_mail() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/dns-query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "Answer": [] })))
        .mount(&server)
        .await;
    run_lua(&format!(
        r#"
        local ev = require("assay.email_verify")
        local c = ev.client({{ doh_url = "{}/dns-query" }})
        local v = c:probe("x@dead.example.com", {{ from = "probe@sender.test" }})
        assert.eq(v.status, "INVALID")
        assert.eq(v.method, "dns:no_mx_no_a")
        assert.eq(v.smtp, nil)
    "#,
        server.uri()
    ))
    .await
    .unwrap();
}

/// A resolver whose MX answer points the probe at a scripted local server.
async fn mx_to_localhost() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/dns-query"))
        .and(query_param("type", "MX"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Answer": [ { "data": "10 127.0.0.1." } ]
        })))
        .mount(&server)
        .await;
    server
}

/// An accepted RCPT stops at PROBABLE: servers accept at RCPT and bounce
/// later, so VERIFIED stays reserved for a delivery that actually happened.
#[tokio::test]
async fn test_probe_maps_smtp_evidence_onto_the_nep_vocabulary() {
    let port = scripted_smtp(vec![
        "220 mx.example.test ESMTP",
        "250 mx.example.test",
        "250 Ok",
        REJECTS_RANDOM,
        "250 Ok",
        "221 Bye",
    ])
    .await;
    let server = mx_to_localhost().await;
    run_lua(&format!(
        r#"
        local ev = require("assay.email_verify")
        local c = ev.client({{ doh_url = "{}/dns-query" }})
        local v = c:probe("info@example.test", {{
          from = "probe@sender.test", port = {port}, greylist_delay_ms = 0,
        }})
        assert.eq(v.status, "PROBABLE")
        assert.eq(v.method, "smtp:accepted")
        assert.eq(v.role, true)
        assert.eq(v.disposable, false)
        assert.eq(v.smtp.deliverable, true)
        assert.eq(v.mx_host, "127.0.0.1")
    "#,
        server.uri()
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn test_probe_reports_a_catch_all_host_as_catch_all() {
    let port = scripted_smtp(vec![
        "220 mx.example.test ESMTP",
        "250 mx.example.test",
        "250 Ok",
        "250 Ok",
    ])
    .await;
    let server = mx_to_localhost().await;
    run_lua(&format!(
        r#"
        local ev = require("assay.email_verify")
        local c = ev.client({{ doh_url = "{}/dns-query" }})
        local v = c:probe("jane@example.test", {{ from = "probe@sender.test", port = {port} }})
        assert.eq(v.status, "CATCH_ALL")
        assert.eq(v.method, "smtp:catch_all")
    "#,
        server.uri()
    ))
    .await
    .unwrap();
}
