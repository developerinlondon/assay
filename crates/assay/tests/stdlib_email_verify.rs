mod common;

use common::run_lua;
use serde_json::json;
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
