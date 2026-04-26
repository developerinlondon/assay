mod common;

use common::run_lua;

#[tokio::test]
async fn test_require_url() {
    let script = r#"
        local url = require("assay.url")
        assert.not_nil(url)
        assert.not_nil(url.encode)
        assert.not_nil(url.encode_form)
        assert.not_nil(url.decode)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_url_encode_space_is_percent_20() {
    let script = r#"
        local url = require("assay.url")
        assert.eq(url.encode("hello world"), "hello%20world")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_url_encode_unreserved_passthrough() {
    let script = r#"
        local url = require("assay.url")
        assert.eq(url.encode("AZaz09-_.~"), "AZaz09-_.~")
    "#;
    run_lua(script).await.unwrap();
}

// Secrets containing &=+% are the original Tailscale footgun.
#[tokio::test]
async fn test_url_encode_form_footgun_chars() {
    let script = r#"
        local url = require("assay.url")
        local s = url.encode("a&b=c+d%e")
        assert.contains(s, "%26")
        assert.contains(s, "%3D")
        assert.contains(s, "%2B")
        assert.contains(s, "%25")
        -- raw chars must NOT remain
        assert.eq(string.find(s, "&", 1, true), nil)
        assert.eq(string.find(s, "=", 1, true), nil)
        assert.eq(string.find(s, "+", 1, true), nil)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_url_round_trip_ascii() {
    let script = r#"
        local url = require("assay.url")
        local cases = { "hello world", "a&b=c+d%e", "x/y?z=1" }
        for _, s in ipairs(cases) do
            assert.eq(url.decode(url.encode(s)), s)
        end
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_url_round_trip_unicode() {
    let script = r#"
        local url = require("assay.url")
        local s = "café"
        assert.eq(url.decode(url.encode(s)), s)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_url_encode_form_basic() {
    let script = r#"
        local url = require("assay.url")
        local body = url.encode_form({ grant_type = "client_credentials", scope = "read write" })
        assert.contains(body, "grant_type=client_credentials")
        assert.contains(body, "scope=read%20write")
        assert.contains(body, "&")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_url_encode_form_escapes_secret_chars() {
    let script = r#"
        local url = require("assay.url")
        local body = url.encode_form({ client_secret = "a&b=c+d%e" })
        assert.contains(body, "client_secret=a%26b%3Dc%2Bd%25e")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_url_encode_form_deterministic_order() {
    let script = r#"
        local url = require("assay.url")
        local body = url.encode_form({ b = "2", a = "1", c = "3" })
        assert.eq(body, "a=1&b=2&c=3")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_url_encode_form_stringifies_values() {
    let script = r#"
        local url = require("assay.url")
        local body = url.encode_form({ n = 42, b = true, f = false })
        assert.contains(body, "n=42")
        assert.contains(body, "b=true")
        assert.contains(body, "f=false")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_url_decode_plus_to_space() {
    let script = r#"
        local url = require("assay.url")
        assert.eq(url.decode("hello+world"), "hello world")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_url_decode_percent_sequences() {
    let script = r#"
        local url = require("assay.url")
        assert.eq(url.decode("a%26b%3Dc"), "a&b=c")
    "#;
    run_lua(script).await.unwrap();
}
