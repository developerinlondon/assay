mod common;

use common::run_lua;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn json_route(server: &MockServer, verb: &str, route: &str, body: serde_json::Value) {
    Mock::given(method(verb))
        .and(path(route))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn test_crm_campaign_reads() {
    let server = MockServer::start().await;
    json_route(
        &server,
        "GET",
        "/api/admin/crm/campaigns",
        serde_json::json!({"campaigns": [{"id": "alpha", "state": "active"}]}),
    )
    .await;
    json_route(
        &server,
        "GET",
        "/api/admin/crm/campaigns/alpha/overview",
        serde_json::json!({"sent": 42, "replies": 3}),
    )
    .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        local list, list_status = c.crm.campaigns:list()
        assert.eq(list_status, 200)
        assert.eq(list.campaigns[1].id, "alpha")
        local overview = c.crm.campaigns:overview("alpha")
        assert.eq(overview.sent, 42)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_crm_campaign_patch_sends_body() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/api/admin/crm/campaigns/alpha"))
        .and(body_json(serde_json::json!({"brief": "shorter"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "alpha"})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        local out, status = c.crm.campaigns:update("alpha", {{ brief = "shorter" }})
        assert.eq(status, 200)
        assert.eq(out.id, "alpha")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_crm_people_list_builds_query() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/crm/people"))
        .and(query_param("campaign", "alpha"))
        .and(query_param("state", "REPLIED"))
        .and(query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"people": []})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        local out = c.crm.people:list({{ campaign = "alpha", state = "REPLIED", limit = 50 }})
        assert.eq(#out.people, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_crm_inbox_draft_lifecycle() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/admin/crm/inbox/drafts"))
        .and(body_json(
            serde_json::json!({"person_id": "p1", "body_text": "hello"}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({"id": "d1"})))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/api/admin/crm/inbox/drafts/d1"))
        .and(body_json(serde_json::json!({"subject": "Re: hello"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "d1"})))
        .mount(&server)
        .await;
    json_route(
        &server,
        "POST",
        "/api/admin/crm/inbox/drafts/d1/send",
        serde_json::json!({"sent": true}),
    )
    .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        local draft, created = c.crm.inbox:draft_create({{ person_id = "p1", body_text = "hello" }})
        assert.eq(created, 201)
        assert.eq(draft.id, "d1")
        c.crm.inbox:draft_update("d1", {{ subject = "Re: hello" }})
        local sent = c.crm.inbox:draft_send("d1")
        assert.eq(sent.sent, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_crm_suppression_lift_encodes_path_segments() {
    let server = MockServer::start().await;
    json_route(
        &server,
        "DELETE",
        "/api/admin/crm/suppressions/address/ada%40example.com",
        serde_json::json!({"removed": 1}),
    )
    .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        local out, status = c.crm.suppressions:lift("address", "ada@example.com")
        assert.eq(status, 200)
        assert.eq(out.removed, 1)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_crm_sends_fleet_and_reports() {
    let server = MockServer::start().await;
    json_route(
        &server,
        "GET",
        "/api/admin/crm/sends/pending",
        serde_json::json!({"sends": [{"id": "s1"}]}),
    )
    .await;
    json_route(
        &server,
        "POST",
        "/api/admin/crm/sends/s1/approve",
        serde_json::json!({"approved": "s1"}),
    )
    .await;
    json_route(
        &server,
        "GET",
        "/api/admin/crm/fleet/summary",
        serde_json::json!({"healthy": 12}),
    )
    .await;
    json_route(
        &server,
        "GET",
        "/api/admin/crm/reports/weekly-note",
        serde_json::json!({"note": "steady"}),
    )
    .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        assert.eq(c.crm.sends:pending().sends[1].id, "s1")
        assert.eq(c.crm.sends:approve("s1").approved, "s1")
        assert.eq(c.crm.fleet:summary().healthy, 12)
        assert.eq(c.crm.reports:weekly_note().note, "steady")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_crm_integrations_account_put() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(
            "/api/admin/crm/integrations/salesforge/accounts/acct-1",
        ))
        .and(body_json(serde_json::json!({"settings": {"region": "eu"}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        local out = c.crm.integrations:account_put("salesforge", "acct-1", {{
            settings = {{ region = "eu" }},
        }})
        assert.eq(out.ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_crm_sequencer_events_query_and_reapply() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/crm/sequencer-events"))
        .and(query_param("campaign", "alpha"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"events": []})))
        .mount(&server)
        .await;
    json_route(
        &server,
        "POST",
        "/api/admin/crm/sequencer-events/e1/reapply",
        serde_json::json!({"reapplied": true}),
    )
    .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        assert.eq(#c.crm:sequencer_events({{ campaign = "alpha" }}).events, 0)
        assert.eq(c.crm:sequencer_event_reapply("e1").reapplied, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_work_items_move_and_unrelate() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/work/items/i1/move"))
        .and(body_json(
            serde_json::json!({"to_status": "in_review", "expected_revision": 4}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id": "i1", "revision": 5})),
        )
        .mount(&server)
        .await;
    json_route(
        &server,
        "DELETE",
        "/api/work/items/i1/relations/i2/blocks",
        serde_json::json!({"removed": true}),
    )
    .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        local moved = c.work.items:move("i1", {{ to_status = "in_review", expected_revision = 4 }})
        assert.eq(moved.revision, 5)
        assert.eq(c.work.items:unrelate("i1", "i2", "blocks").removed, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_work_projects_plans_and_ready() {
    let server = MockServer::start().await;
    json_route(
        &server,
        "GET",
        "/api/work/projects/pr1/board",
        serde_json::json!({"columns": [{"key": "ready"}]}),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/work/plans/pl1/decide"))
        .and(body_json(serde_json::json!({"accept": true})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"state": "accepted"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/work/ready"))
        .and(query_param("project", "pr1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        assert.eq(c.work.projects:board("pr1").columns[1].key, "ready")
        assert.eq(c.work.plans:decide("pl1", {{ accept = true }}).state, "accepted")
        assert.eq(#c.work:ready({{ project = "pr1" }}).items, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_request_escape_hatch_returns_body_and_status() {
    let server = MockServer::start().await;
    json_route(
        &server,
        "GET",
        "/api/admin/crm/overview",
        serde_json::json!({"campaigns": 3}),
    )
    .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        local body, status = c:request("GET", "/api/admin/crm/overview")
        assert.eq(status, 200)
        assert.eq(body.campaigns, 3)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_non_2xx_returns_error_body_without_throwing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/admin/crm/campaigns"))
        .respond_with(
            ResponseTemplate::new(409).set_body_json(serde_json::json!({"error": "id taken"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/admin/crm/claims"))
        .respond_with(ResponseTemplate::new(503).set_body_string("upstream down"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        local body, status = c.crm.campaigns:create({{ name = "Alpha" }})
        assert.eq(status, 409)
        assert.eq(body.error, "id taken")
        local raw, raw_status = c.crm.claims:list()
        assert.eq(raw_status, 503)
        assert.eq(raw, "upstream down")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_client_falls_back_to_env_url_and_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/work/repositories"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer nck_from_env",
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"repositories": []})),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        env.set("NEUTRON_URL", "{}/")
        env.set("NEUTRON_TOKEN", "nck_from_env")
        local neutron = require("assay.neutron")
        local c = neutron.client()
        assert.eq(#c.work:repositories().repositories, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_claims_approve_sends_review_by() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/admin/crm/claims/cl1/approve"))
        .and(body_json(serde_json::json!({"review_by": "2027-03-01"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        local out = c.crm.claims:approve("cl1", {{ review_by = "2027-03-01" }})
        assert.eq(out.ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_people_imports_passes_campaign_query() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/crm/people/imports"))
        .and(query_param("campaign", "alpha"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"batches": []})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        assert.eq(#c.crm.people:imports({{ campaign = "alpha" }}).batches, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_people_import_dry_runs_unless_told_otherwise() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/admin/crm/people/import"))
        .and(query_param("dry_run", "1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"dry_run": true})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/admin/crm/people/import"))
        .and(wiremock::matchers::query_param_is_missing("dry_run"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"dry_run": false})),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        local body = {{ campaign = "alpha", csv = "email\nada@example.com" }}
        assert.eq(c.crm.people:import(body).dry_run, true)
        assert.eq(c.crm.people:import(body, {{}}).dry_run, true)
        assert.eq(c.crm.people:import(body, {{ dry_run = true }}).dry_run, true)
        assert.eq(c.crm.people:import(body, {{ dry_run = false }}).dry_run, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_opts_headers_ride_every_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/crm/overview"))
        .and(wiremock::matchers::header("x-neutron-internal", "1"))
        .and(wiremock::matchers::header("x-neutron-token", "int_tok"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer nck_test",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{
            token = "nck_test",
            headers = {{ ["x-neutron-internal"] = "1", ["x-neutron-token"] = "int_tok" }},
        }})
        assert.eq(c.crm:overview().ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_opts_headers_cannot_replace_the_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/crm/overview"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer nck_test",
        ))
        .and(wiremock::matchers::header("x-trace", "t1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{
            token = "nck_test",
            headers = {{ Authorization = "Bearer stolen", ["x-trace"] = "t1" }},
        }})
        assert.eq(c.crm:overview().ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_opts_headers_rejects_a_non_table() {
    let script = r#"
        local neutron = require("assay.neutron")
        local ok, err = pcall(function()
            return neutron.client("http://x.example", { token = "t", headers = "nope" })
        end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "opts.headers must be a table")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_proposals_list_accept_and_decline() {
    let server = MockServer::start().await;
    json_route(
        &server,
        "GET",
        "/api/admin/crm/proposals",
        serde_json::json!({"proposals": [{"id": "p1"}]}),
    )
    .await;
    json_route(
        &server,
        "POST",
        "/api/admin/crm/proposals/p1/accept",
        serde_json::json!({"ok": true}),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/admin/crm/proposals/p2/decline"))
        .respond_with(
            ResponseTemplate::new(409)
                .set_body_json(serde_json::json!({"error": "already accepted"})),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        assert.eq(c.crm.proposals:list().proposals[1].id, "p1")
        assert.eq(c.crm.proposals:accept("p1").ok, true)
        local body, status = c.crm.proposals:decline("p2")
        assert.eq(status, 409)
        assert.eq(body.error, "already accepted")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_integration_account_lifecycle() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/admin/crm/integrations/salesforge/accounts"))
        .and(body_json(serde_json::json!({"label": "EU"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({"added": "a2"})))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/api/admin/crm/integrations/salesforge/accounts/a2"))
        .and(body_json(serde_json::json!({"fleet_label": "fleet-002"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server)
        .await;
    json_route(
        &server,
        "POST",
        "/api/admin/crm/integrations/salesforge/accounts/a2/webhook-secret",
        serde_json::json!({"rotated": true}),
    )
    .await;
    json_route(
        &server,
        "DELETE",
        "/api/admin/crm/integrations/salesforge/accounts/a2/secrets/api_key",
        serde_json::json!({"cleared": true}),
    )
    .await;
    json_route(
        &server,
        "DELETE",
        "/api/admin/crm/integrations/salesforge/accounts/a2",
        serde_json::json!({"removed": true}),
    )
    .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        local ig = c.crm.integrations
        local created, status = ig:account_create("salesforge", {{ label = "EU" }})
        assert.eq(status, 201)
        assert.eq(created.added, "a2")
        assert.eq(ig:account_update("salesforge", "a2", {{ fleet_label = "fleet-002" }}).ok, true)
        assert.eq(ig:account_webhook_secret("salesforge", "a2").rotated, true)
        assert.eq(ig:account_secret_delete("salesforge", "a2", "api_key").cleared, true)
        assert.eq(ig:account_delete("salesforge", "a2").removed, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_parameters_the_old_signatures_could_not_reach() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/admin/crm/domains/mail.example.com/pause"))
        .and(body_json(serde_json::json!({"reason": "bounces"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"paused": true})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/admin/crm/domains/mail.example.com/recheck"))
        .and(body_json(serde_json::json!({"dkim_selector": "s1"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/admin/crm/claims"))
        .and(query_param("citable", "1"))
        .and(query_param("locale", "en-GB"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"claims": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/admin/crm/reports/conversion"))
        .and(query_param("since", "2026-01-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"rows": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/admin/crm/reports/weekly-note"))
        .and(query_param("week", "2026-09-14"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"note": "ok"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/work/projects/pr1/board"))
        .and(query_param("sprint", "current"))
        .and(query_param("done", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"columns": []})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/work/sprints/s1/close"))
        .and(body_json(serde_json::json!({"carry_to": "next"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"closed": true})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        assert.eq(c.crm.domains:pause("mail.example.com", {{ reason = "bounces" }}).paused, true)
        assert.eq(c.crm.domains:recheck("mail.example.com", {{ dkim_selector = "s1" }}).ok, true)
        assert.eq(#c.crm.claims:list({{ citable = "1", locale = "en-GB" }}).claims, 0)
        assert.eq(#c.crm.reports:conversion({{ since = "2026-01-01" }}).rows, 0)
        assert.eq(c.crm.reports:weekly_note({{ week = "2026-09-14" }}).note, "ok")
        local board = c.work.projects:board("pr1", {{ sprint = "current", done = "1" }})
        assert.eq(#board.columns, 0)
        assert.eq(c.work.sprints:close("s1", {{ carry_to = "next" }}).closed, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_move_unassign_sprint_puts_a_real_null_on_the_wire() {
    let server = MockServer::start().await;
    // Both matchers on purpose: the raw substring proves a JSON null literally
    // reached the wire, the parsed shape proves nothing else was mangled.
    Mock::given(method("POST"))
        .and(path("/api/work/items/i1/move"))
        .and(wiremock::matchers::body_string_contains(
            "\"sprint_id\":null",
        ))
        .and(body_json(serde_json::json!({
            "to_status": "ready",
            "expected_revision": 7,
            "sprint_id": null,
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"moved": true})))
        .mount(&server)
        .await;
    // A body carrying only the null, to cover the empty-object seam.
    Mock::given(method("POST"))
        .and(path("/api/work/items/i2/move"))
        .and(body_json(serde_json::json!({"sprint_id": null})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"moved": true})))
        .mount(&server)
        .await;
    // Without the option the key is simply absent, which keeps the sprint.
    Mock::given(method("POST"))
        .and(path("/api/work/items/i3/move"))
        .and(body_json(
            serde_json::json!({"to_status": "ready", "expected_revision": 7}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"moved": true})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local c = neutron.client("{}", {{ token = "nck_test" }})
        local unassign = {{ unassign_sprint = true }}

        -- a camelCase sprintId in the body must not outrank the null
        local body = {{ to_status = "ready", expected_revision = 7, sprintId = "s9" }}
        assert.eq(c.work.items:move("i1", body, unassign).moved, true)

        assert.eq(c.work.items:move("i2", {{}}, unassign).moved, true)
        assert.eq(c.work.items:move("i2", nil, unassign).moved, true)

        -- sprint_id = nil is an absent key, not a null: the sprint is kept
        local kept = {{ to_status = "ready", expected_revision = 7, sprint_id = nil }}
        assert.eq(c.work.items:move("i3", kept).moved, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
