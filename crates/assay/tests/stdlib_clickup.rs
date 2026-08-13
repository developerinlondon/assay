mod common;

use common::run_lua;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn mock_json(server: &MockServer, verb: &str, route: &str, body: serde_json::Value) {
    Mock::given(method(verb))
        .and(path(route.to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

fn script(uri: &str, body: &str) -> String {
    format!(
        r#"
        local clickup = require("assay.clickup")
        local c = clickup.client({{ token = "k", base_url = "{uri}" }})
        {body}
    "#
    )
}

#[tokio::test]
async fn test_require_clickup() {
    let body = r#"
        local clickup = require("assay.clickup")
        assert.not_nil(clickup.client)
        assert.not_nil(clickup.all_tasks)
        assert.not_nil(clickup.find_task_by_name)
        assert.not_nil(clickup.ensure_task)
        assert.not_nil(clickup.resolve_team)
        assert.not_nil(clickup.resolve_member)
        assert.not_nil(clickup.rich)
        assert.not_nil(clickup.comment_payload)
    "#;
    run_lua(body).await.unwrap();
}

#[tokio::test]
async fn test_clickup_client_sections() {
    let body = r#"
        for _, section in ipairs({
            "teams", "spaces", "folders", "lists", "tasks",
            "comments", "goals", "fields", "time", "docs",
        }) do
            assert.not_nil(c[section], "missing section: " .. section)
        end
        assert.eq(type(c.discover), "function")
    "#;
    run_lua(&script("http://example.invalid", body))
        .await
        .unwrap();
}

// ClickUp rejects a "Bearer " prefix on personal pk_ tokens. Matching the header
// on its exact value fails the request if the module ever adds one.
#[tokio::test]
async fn test_clickup_auth_header_is_raw_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/team"))
        .and(header("Authorization", "pk_test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "teams": [{"id": "9001", "name": "Acme"}]
        })))
        .mount(&server)
        .await;

    let body = format!(
        r#"
        local clickup = require("assay.clickup")
        local c = clickup.client({{ token = "pk_test_token", base_url = "{}" }})
        local teams = c.teams:list()
        assert.eq(#teams, 1)
        assert.eq(teams[1].name, "Acme")
    "#,
        server.uri()
    );
    run_lua(&body).await.unwrap();
}

#[tokio::test]
async fn test_clickup_token_from_env() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/team"))
        .and(header("Authorization", "pk_from_env"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "teams": [{"id": "1", "name": "Env"}]
        })))
        .mount(&server)
        .await;

    let body = format!(
        r#"
        env.set("CLICKUP_TOKEN", "pk_from_env")
        local clickup = require("assay.clickup")
        local c = clickup.client({{ base_url = "{}" }})
        assert.eq(c.teams:list()[1].name, "Env")
    "#,
        server.uri()
    );
    run_lua(&body).await.unwrap();
}

#[tokio::test]
async fn test_clickup_tasks_list_and_create() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        "GET",
        "/v2/list/L1/task",
        serde_json::json!({"tasks": [{"id": "t1", "name": "First"}], "last_page": true}),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/v2/list/L1/task"))
        .and(body_json(serde_json::json!({"name": "New task"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "t2", "name": "New task"
        })))
        .mount(&server)
        .await;

    let body = r#"
        local tasks = c.tasks:list("L1")
        assert.eq(#tasks, 1)
        assert.eq(tasks[1].id, "t1")
        assert.eq(c.tasks:create("L1", { name = "New task" }).id, "t2")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

// List-valued filters must repeat as `statuses[]=`, not join with commas.
#[tokio::test]
async fn test_clickup_array_query_params() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/list/L1/task"))
        .and(query_param("statuses[]", "in progress"))
        .and(query_param("page", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "tasks": [{"id": "t9", "name": "Filtered"}],
            "last_page": true
        })))
        .mount(&server)
        .await;

    let body = r#"
        local tasks = c.tasks:list("L1", { statuses = { "in progress" }, page = 0 })
        assert.eq(#tasks, 1)
        assert.eq(tasks[1].id, "t9")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_clickup_all_tasks_walks_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/list/L1/task"))
        .and(query_param("page", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "tasks": [{"id": "t1"}, {"id": "t2"}],
            "last_page": false
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/list/L1/task"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "tasks": [{"id": "t3"}],
            "last_page": true
        })))
        .mount(&server)
        .await;

    let body = r#"
        local all = clickup.all_tasks(c, "L1")
        assert.eq(#all, 3)
        assert.eq(all[3].id, "t3")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

// A missing `last_page` must end the walk rather than page forever.
#[tokio::test]
async fn test_clickup_all_tasks_stops_without_last_page_flag() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        "GET",
        "/v2/list/L1/task",
        serde_json::json!({"tasks": [{"id": "only"}]}),
    )
    .await;

    let body = r#"
        assert.eq(#clickup.all_tasks(c, "L1"), 1)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_clickup_all_tasks_respects_max_pages() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        "GET",
        "/v2/list/L1/task",
        serde_json::json!({"tasks": [{"id": "loop"}], "last_page": false}),
    )
    .await;

    let body = r#"
        assert.eq(#clickup.all_tasks(c, "L1", { max_pages = 3 }), 3)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_clickup_ensure_task_is_idempotent() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        "GET",
        "/v2/list/L1/task",
        serde_json::json!({"tasks": [{"id": "existing", "name": "Ship it"}], "last_page": true}),
    )
    .await;

    // No POST mock is registered: creating instead of reusing would fail the call.
    let body = r#"
        assert.eq(clickup.ensure_task(c, "L1", { name = "Ship it" }).id, "existing")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_clickup_ensure_task_creates_when_absent() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        "GET",
        "/v2/list/L1/task",
        serde_json::json!({"tasks": [{"id": "other", "name": "Something else"}], "last_page": true}),
    )
    .await;
    mock_json(
        &server,
        "POST",
        "/v2/list/L1/task",
        serde_json::json!({"id": "fresh", "name": "Ship it"}),
    )
    .await;

    let body = r#"
        assert.eq(clickup.ensure_task(c, "L1", { name = "Ship it" }).id, "fresh")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_clickup_ensure_task_requires_name() {
    let body = r#"
        local ok, err = pcall(function()
            return clickup.ensure_task(c, "L1", { description = "no name" })
        end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "ensure_task requires spec.name")
    "#;
    run_lua(&script("http://example.invalid", body))
        .await
        .unwrap();
}

#[tokio::test]
async fn test_clickup_custom_field_set_wraps_value() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/v2/task/T1/field/F1"))
        .and(body_json(serde_json::json!({"value": 42})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let body = r#"
        assert.eq(c.fields:set("T1", "F1", 42), true)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_clickup_goals_use_singular_paths() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        "GET",
        "/v2/team/9001/goal",
        serde_json::json!({"goals": [{"id": "g1", "name": "Q3 revenue"}]}),
    )
    .await;
    mock_json(
        &server,
        "GET",
        "/v2/goal/g1",
        serde_json::json!({"goal": {"id": "g1", "name": "Q3 revenue"}}),
    )
    .await;
    mock_json(
        &server,
        "POST",
        "/v2/team/9001/goal",
        serde_json::json!({"goal": {"id": "g2", "name": "Q4 revenue"}}),
    )
    .await;
    mock_json(
        &server,
        "PUT",
        "/v2/goal/g2",
        serde_json::json!({"goal": {"id": "g2", "name": "Q4 revenue revised"}}),
    )
    .await;

    let body = r#"
        assert.eq(#c.goals:list("9001"), 1)
        assert.eq(c.goals:get("g1").name, "Q3 revenue")
        assert.eq(c.goals:create("9001", { name = "Q4 revenue" }).id, "g2")
        assert.eq(c.goals:update("g2", { name = "Q4 revenue revised" }).name, "Q4 revenue revised")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

// Docs are the one resource on v3, and v3 roots them at /workspaces/{id}.
#[tokio::test]
async fn test_clickup_docs_use_v3_workspace_path() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        "GET",
        "/v3/workspaces/9001/docs",
        serde_json::json!({"docs": [{"id": "d1", "name": "Sprint notes"}]}),
    )
    .await;
    mock_json(
        &server,
        "POST",
        "/v3/workspaces/9001/docs/d1/pages",
        serde_json::json!({"id": "p1", "name": "Retro"}),
    )
    .await;

    let body = r#"
        assert.eq(c.docs:search("9001").docs[1].id, "d1")
        assert.eq(c.docs:create_page("9001", "d1", { name = "Retro" }).id, "p1")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_clickup_stop_timer_posts_without_entry_id() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        "POST",
        "/v2/team/9001/time_entries/stop",
        serde_json::json!({"data": {"id": "e1"}}),
    )
    .await;

    let body = r#"
        assert.not_nil(c.time:stop("9001"))
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_clickup_missing_resource_returns_nil() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/task/GONE"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let body = r#"
        assert.eq(c.tasks:get("GONE"), nil)
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_clickup_error_status_raises() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/team"))
        .respond_with(ResponseTemplate::new(401).set_body_string("token invalid"))
        .mount(&server)
        .await;

    let body = r#"
        local ok, err = pcall(function() return c.teams:list() end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "clickup: GET /team HTTP 401")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_clickup_resolve_team_single_and_named() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        "GET",
        "/v2/team",
        serde_json::json!({"teams": [{"id": "1", "name": "Alpha"}, {"id": "2", "name": "Beta"}]}),
    )
    .await;

    let body = r#"
        assert.eq(clickup.resolve_team(c, "Beta").id, "2")

        local ok, err = pcall(function() return clickup.resolve_team(c) end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "pass a name to disambiguate")

        local missing_ok, missing_err = pcall(function()
            return clickup.resolve_team(c, "Nope")
        end)
        assert.eq(missing_ok, false)
        assert.contains(tostring(missing_err), "no workspace named Nope")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_clickup_lists_and_spaces_paths() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        "GET",
        "/v2/team/9001/space",
        serde_json::json!({"spaces": [{"id": "s1", "name": "Product"}]}),
    )
    .await;
    mock_json(
        &server,
        "GET",
        "/v2/space/s1/folder",
        serde_json::json!({"folders": [{"id": "f1", "name": "Sprints"}]}),
    )
    .await;
    mock_json(
        &server,
        "GET",
        "/v2/folder/f1/list",
        serde_json::json!({"lists": [{"id": "l1", "name": "Sprint 1"}]}),
    )
    .await;
    mock_json(
        &server,
        "GET",
        "/v2/space/s1/list",
        serde_json::json!({"lists": [{"id": "l2", "name": "Backlog"}]}),
    )
    .await;

    let body = r#"
        assert.eq(c.spaces:list("9001")[1].id, "s1")
        assert.eq(c.folders:list("s1")[1].id, "f1")
        assert.eq(c.lists:list("f1")[1].id, "l1")
        assert.eq(c.lists:folderless("s1")[1].id, "l2")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

#[tokio::test]
async fn test_clickup_comment_accepts_bare_string() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/task/T1/comment"))
        .and(body_json(serde_json::json!({"comment_text": "done"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "c1"})))
        .mount(&server)
        .await;

    let body = r#"
        assert.eq(c.comments:create("T1", "done").id, "c1")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

// The delta is the whole point: line formatting rides the newline op, a mention
// is a `tag` block carrying the numeric id, and no markdown reaches the wire.
#[tokio::test]
async fn test_clickup_rich_comment_builds_a_delta() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/task/T1/comment"))
        .and(body_json(serde_json::json!({
            "notify_all": true,
            "comment": [
                {"text": "Shipped", "attributes": {"bold": true}},
                {"text": "\n"},
                {"text": "see "},
                {"text": "the docs", "attributes": {"link": "https://example.com"}},
                {"text": "\n", "attributes": {"list": "bullet"}},
                {"text": "docs/hextra/content", "attributes": {"code": true}},
                {"text": "\n", "attributes": {"list": "ordered"}},
                {"type": "tag", "user": {"id": 113633770}, "text": "@Bharat Paryani"},
                {"text": " over to you."},
                {"text": "\n"}
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "c9"})))
        .mount(&server)
        .await;

    let body = r#"
        local rich = clickup.rich()
            :bold("Shipped"):br()
            :text("see "):link("the docs", "https://example.com"):bullet()
            :code("docs/hextra/content"):number()
            :mention({ id = 113633770, username = "Bharat Paryani" })
            :text(" over to you."):br()

        assert.eq(c.comments:create("T1", rich, { notify_all = true }).id, "c9")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}

// A mention given a bare id still tags: the id notifies, the label is cosmetic.
#[tokio::test]
async fn test_clickup_mention_accepts_a_bare_id() {
    let body = r#"
        local ops = clickup.rich():mention(42):build().comment
        assert.eq(ops[1].type, "tag")
        assert.eq(ops[1].user.id, 42)
        assert.eq(ops[1].text, "@42")

        local ok, err = pcall(function() return clickup.rich():mention({}) end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "mention needs a user id")
    "#;
    run_lua(&script("http://example.invalid", body)).await.unwrap();
}

// `extra` must not write through into the caller's table.
#[tokio::test]
async fn test_clickup_comment_payload_does_not_mutate_its_input() {
    let body = r#"
        local original = { comment_text = "note" }
        local payload = clickup.comment_payload(original)
        payload.notify_all = true
        assert.eq(original.notify_all, nil)
        assert.eq(payload.comment_text, "note")

        assert.eq(clickup.comment_payload("plain").comment_text, "plain")
    "#;
    run_lua(&script("http://example.invalid", body)).await.unwrap();
}

#[tokio::test]
async fn test_clickup_resolve_member_by_name_email_and_ambiguity() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        "GET",
        "/v2/team",
        serde_json::json!({"teams": [{
            "id": "90182966627",
            "name": "Acme",
            "members": [
                {"user": {"id": 1, "username": "Bharat Paryani", "email": "dev@acme.test"}},
                {"user": {"id": 2, "username": "Bharti Rao", "email": "ops@acme.test"}}
            ]
        }]}),
    )
    .await;

    let body = r#"
        assert.eq(clickup.resolve_member(c, "90182966627", "Bharat Paryani").id, 1)
        assert.eq(clickup.resolve_member(c, "90182966627", "ops@acme.test").id, 2)
        assert.eq(clickup.resolve_member(c, "90182966627", "paryani").id, 1)

        local ok, err = pcall(function()
            return clickup.resolve_member(c, "90182966627", "bhar")
        end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "2 members match")

        local missing_ok, missing_err = pcall(function()
            return clickup.resolve_member(c, "90182966627", "nobody")
        end)
        assert.eq(missing_ok, false)
        assert.contains(tostring(missing_err), "no member of workspace")
    "#;
    run_lua(&script(&server.uri(), body)).await.unwrap();
}
