mod common;

use common::run_lua;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_openclaw() {
    let script = r#"
        local mod = require("assay.openclaw")
        assert.not_nil(mod)
        assert.not_nil(mod.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_openclaw_invoke() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tools/invoke"))
        .and(header("Authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": "ok",
            "output": "tool executed"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local openclaw = require("assay.openclaw")
        local c = openclaw.client("{}", {{ token = "test-token" }})
        local result = c.tools:invoke("mytool", "run", {{ key = "value" }})
        assert.eq(result.result, "ok")
        assert.eq(result.output, "tool executed")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_openclaw_send() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tools/invoke"))
        .and(body_json(serde_json::json!({
            "tool": "message",
            "action": "send",
            "args": {
                "channel": "slack",
                "target": "#alerts",
                "message": "hello world"
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "sent": true
        })))
        .mount(&server)
        .await;

    let script = format!(
        r##"
        local openclaw = require("assay.openclaw")
        local c = openclaw.client("{}", {{ token = "t" }})
        local result = c.messaging:send("slack", "#alerts", "hello world")
        assert.eq(result.sent, true)
        "##,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_openclaw_notify() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tools/invoke"))
        .and(body_json(serde_json::json!({
            "tool": "message",
            "action": "send",
            "args": {
                "target": "ops",
                "message": "deploy finished"
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "queued": true
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local openclaw = require("assay.openclaw")
        local c = openclaw.client("{}", {{ token = "t" }})
        local result = c.messaging:notify("ops", "deploy finished")
        assert.eq(result.queued, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_openclaw_state_get_set() {
    let script = r#"
        local openclaw = require("assay.openclaw")
        local tmpdir = fs.tempdir()
        local c = openclaw.client("http://localhost:1", { token = "t", state_dir = tmpdir })

        -- state_get returns nil for missing key
        local val = c.state:get("nonexistent")
        assert.eq(val, nil)

        -- state_set and state_get round-trip
        c.state:set("mykey", { count = 42, name = "test" })
        local got = c.state:get("mykey")
        assert.eq(got.count, 42)
        assert.eq(got.name, "test")

        -- overwrite
        c.state:set("mykey", { count = 99 })
        local got2 = c.state:get("mykey")
        assert.eq(got2.count, 99)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_openclaw_diff() {
    let script = r#"
        local openclaw = require("assay.openclaw")
        local tmpdir = fs.tempdir()
        local c = openclaw.client("http://localhost:1", { token = "t", state_dir = tmpdir })

        -- First diff: before is nil, after is new value
        local d = c.state:diff("counter", { value = 1 })
        assert.eq(d.changed, true)
        assert.eq(d.before, nil)
        assert.eq(d.after.value, 1)

        -- Same value: no change
        local d2 = c.state:diff("counter", { value = 1 })
        assert.eq(d2.changed, false)
        assert.eq(d2.before.value, 1)
        assert.eq(d2.after.value, 1)

        -- Different value: changed
        local d3 = c.state:diff("counter", { value = 2 })
        assert.eq(d3.changed, true)
        assert.eq(d3.before.value, 1)
        assert.eq(d3.after.value, 2)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_openclaw_llm_task() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tools/invoke"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "response": "The answer is 42",
            "model": "claude-sonnet"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local openclaw = require("assay.openclaw")
        local c = openclaw.client("{}", {{ token = "t" }})
        local result = c.llm:task("What is the meaning of life?", {{
            model = "claude-sonnet",
            temperature = 0.5,
        }})
        assert.eq(result.response, "The answer is 42")
        assert.eq(result.model, "claude-sonnet")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_openclaw_cron_add() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tools/invoke"))
        .and(body_json(serde_json::json!({
            "tool": "cron",
            "action": "add",
            "args": {
                "job": {
                    "schedule": "0 * * * *",
                    "task": "health-check"
                }
            }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "cron-1",
            "created": true
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local openclaw = require("assay.openclaw")
        local c = openclaw.client("{}", {{ token = "t" }})
        local result = c.cron:add({{ schedule = "0 * * * *", task = "health-check" }})
        assert.eq(result.id, "cron-1")
        assert.eq(result.created, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_openclaw_cron_list() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tools/invoke"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jobs": [
                { "id": "job-1", "schedule": "0 * * * *", "task": "health-check" },
                { "id": "job-2", "schedule": "0 0 * * *", "task": "daily-report" }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local openclaw = require("assay.openclaw")
        local c = openclaw.client("{}", {{ token = "t" }})
        local result = c.cron:list()
        assert.eq(#result.jobs, 2)
        assert.eq(result.jobs[1].id, "job-1")
        assert.eq(result.jobs[2].task, "daily-report")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_openclaw_spawn() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tools/invoke"))
        .and(body_json(serde_json::json!({
            "tool": "sessions_spawn",
            "action": "invoke",
            "args": {
                "task": "draft summary",
                "model": "claude-sonnet",
                "timeout": 30
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "session_id": "ses-123",
            "status": "started"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local openclaw = require("assay.openclaw")
        local c = openclaw.client("{}", {{ token = "t" }})
        local result = c.sessions:spawn("draft summary", {{ model = "claude-sonnet", timeout = 30 }})
        assert.eq(result.session_id, "ses-123")
        assert.eq(result.status, "started")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_openclaw_approve_noninteractive_errors() {
    let script = r#"
        local openclaw = require("assay.openclaw")
        local c = openclaw.client("http://localhost:1", { token = "t" })
        local ok, err = pcall(function()
            c.gates:approve("Deploy now?", { env = "prod" })
        end)
        assert.eq(ok, false)
        assert.contains(err, "openclaw: approval_required:")
        assert.contains(err, '"prompt":"Deploy now?"')
    "#;
    run_lua(script).await.unwrap();
}
