mod common;

use common::run_lua;

#[tokio::test]
async fn test_require_email_triage() {
    let script = r#"
        local mod = require("assay.email_triage")
        assert.not_nil(mod)
        assert.not_nil(mod.categorize)
        assert.not_nil(mod.categorize_llm)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_categorize_needs_action() {
    let script = r#"
        local triage = require("assay.email_triage")
        local result = triage.categorize({
            { from = "ceo@example.com", subject = "Action required: budget sign-off" },
            { from = "pm@example.com", subject = "URGENT deadline changed" },
        })
        assert.eq(#result.needs_action, 2)
        assert.eq(#result.needs_reply, 0)
        assert.eq(#result.fyi, 0)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_categorize_needs_reply() {
    let script = r#"
        local triage = require("assay.email_triage")
        local result = triage.categorize({
            { from = "alice@example.com", subject = "Can we meet tomorrow?" },
            { from = "bob@example.com", subject = "Question about rollout" },
        })
        assert.eq(#result.needs_reply, 2)
        assert.eq(result.needs_reply[1].from, "alice@example.com")
        assert.eq(#result.needs_action, 0)
        assert.eq(#result.fyi, 0)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_categorize_fyi() {
    let script = r#"
        local triage = require("assay.email_triage")
        local result = triage.categorize({
            { from = "noreply@example.com", subject = "Your weekly report" },
            { from = "alerts@example.com", subject = "Automated deployment notice", automated = true },
        })
        assert.eq(#result.fyi, 2)
        assert.eq(#result.needs_reply, 0)
        assert.eq(#result.needs_action, 0)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_categorize_empty() {
    let script = r#"
        local triage = require("assay.email_triage")
        local result = triage.categorize({})
        assert.eq(#result.needs_reply, 0)
        assert.eq(#result.needs_action, 0)
        assert.eq(#result.fyi, 0)
    "#;
    run_lua(script).await.unwrap();
}
