mod common;

use common::run_lua;

#[tokio::test]
async fn test_require_ansi() {
    let script = r#"
        local mod = require("assay.ansi")
        assert.not_nil(mod)
        assert.not_nil(mod.to_html)
        assert.not_nil(mod.strip)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_strip_sgr_color() {
    let script = r#"
        local ansi = require("assay.ansi")
        assert.eq(ansi.strip("\27[32mhi\27[0m"), "hi")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_strip_non_sgr_csi() {
    let script = r#"
        local ansi = require("assay.ansi")
        assert.eq(ansi.strip("\27[2K"), "")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_strip_preserves_plain_text() {
    let script = r#"
        local ansi = require("assay.ansi")
        assert.eq(ansi.strip("plain text"), "plain text")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_strip_empty() {
    let script = r#"
        local ansi = require("assay.ansi")
        assert.eq(ansi.strip(""), "")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_to_html_escapes_unsafe_chars() {
    let script = r#"
        local ansi = require("assay.ansi")
        assert.eq(ansi.to_html("&<>"), "&amp;&lt;&gt;")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_to_html_fg_color() {
    let script = r#"
        local ansi = require("assay.ansi")
        local out = ansi.to_html("\27[32mok\27[0m")
        assert.not_nil(string.find(out, '<span class="ansi-fg-32">ok</span>', 1, true))
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_to_html_bold() {
    let script = r#"
        local ansi = require("assay.ansi")
        local out = ansi.to_html("\27[1mB\27[0m")
        assert.not_nil(string.find(out, '<span class="ansi-bold">B</span>', 1, true))
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_to_html_default_fg_closes_span() {
    let script = r#"
        local ansi = require("assay.ansi")
        local out = ansi.to_html("\27[36mreq\27[39m: x")
        assert.not_nil(string.find(out, '<span class="ansi-fg-36">req</span>: x', 1, true))
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_to_html_unknown_code_dropped() {
    let script = r#"
        local ansi = require("assay.ansi")
        local out = ansi.to_html("\27[99munknown\27[0m")
        assert.eq(out, "unknown")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_to_html_strips_non_sgr_csi() {
    let script = r#"
        local ansi = require("assay.ansi")
        assert.eq(ansi.to_html("\27[2Kafter erase"), "after erase")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_to_html_empty() {
    let script = r#"
        local ansi = require("assay.ansi")
        assert.eq(ansi.to_html(""), "")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_to_html_no_unclosed_spans_at_eol() {
    let script = r#"
        local ansi = require("assay.ansi")
        local out = ansi.to_html("\27[31mred")
        local _, opens = string.gsub(out, "<span", "")
        local _, closes = string.gsub(out, "</span>", "")
        assert.eq(opens, closes)
    "#;
    run_lua(script).await.unwrap();
}
