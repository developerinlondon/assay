mod common;

use common::run_lua;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_github() {
    let script = r#"
        local mod = require("assay.github")
        assert.not_nil(mod)
        assert.not_nil(mod.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_github_pr_view() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octocat/hello-world/pulls/42"))
        .and(header("Authorization", "Bearer ghp_test123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "number": 42,
            "title": "Fix bug",
            "state": "open",
            "user": { "login": "octocat" },
            "mergeable": true
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local github = require("assay.github")
        local c = github.client({{ token = "ghp_test123", base_url = "{}" }})
        local pr = c.pulls:get("octocat/hello-world", 42)
        assert.eq(pr.number, 42)
        assert.eq(pr.title, "Fix bug")
        assert.eq(pr.state, "open")
        assert.eq(pr.user.login, "octocat")
        assert.eq(pr.mergeable, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_github_pr_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octocat/hello-world/pulls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "number": 1, "title": "First PR", "state": "open" },
            { "number": 2, "title": "Second PR", "state": "closed" }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local github = require("assay.github")
        local c = github.client({{ base_url = "{}" }})
        local prs = c.pulls:list("octocat/hello-world")
        assert.eq(#prs, 2)
        assert.eq(prs[1].number, 1)
        assert.eq(prs[1].title, "First PR")
        assert.eq(prs[2].state, "closed")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_github_pr_reviews() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octocat/hello-world/pulls/42/reviews"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "id": 1, "state": "APPROVED", "user": { "login": "alice" } },
            { "id": 2, "state": "COMMENTED", "user": { "login": "bob" } }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local github = require("assay.github")
        local c = github.client({{ base_url = "{}" }})
        local reviews = c.pulls:reviews("octocat/hello-world", 42)
        assert.eq(#reviews, 2)
        assert.eq(reviews[1].state, "APPROVED")
        assert.eq(reviews[2].user.login, "bob")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_github_pr_merge() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/repos/octocat/hello-world/pulls/42/merge"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "merged": true,
            "sha": "abc123"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local github = require("assay.github")
        local c = github.client({{ token = "ghp_test", base_url = "{}" }})
        local result = c.pulls:merge("octocat/hello-world", 42, {{ merge_method = "squash" }})
        assert.eq(result.merged, true)
        assert.eq(result.sha, "abc123")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_github_issue_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octocat/hello-world/issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "number": 10, "title": "Bug report", "state": "open", "labels": [{"name": "bug"}] },
            { "number": 11, "title": "Feature request", "state": "open", "labels": [{"name": "enhancement"}] }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local github = require("assay.github")
        local c = github.client({{ base_url = "{}" }})
        local issues = c.issues:list("octocat/hello-world")
        assert.eq(#issues, 2)
        assert.eq(issues[1].number, 10)
        assert.eq(issues[1].title, "Bug report")
        assert.eq(issues[1].labels[1].name, "bug")
        assert.eq(issues[2].title, "Feature request")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_github_issue_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octocat/hello-world/issues/10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "number": 10,
            "title": "Bug report",
            "state": "open",
            "user": { "login": "reporter" }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local github = require("assay.github")
        local c = github.client({{ base_url = "{}" }})
        local issue = c.issues:get("octocat/hello-world", 10)
        assert.eq(issue.number, 10)
        assert.eq(issue.title, "Bug report")
        assert.eq(issue.user.login, "reporter")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_github_issue_create() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/octocat/hello-world/issues"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 42,
            "title": "New issue",
            "body": "Description here",
            "state": "open",
            "html_url": "https://github.com/octocat/hello-world/issues/42"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local github = require("assay.github")
        local c = github.client({{ token = "ghp_test", base_url = "{}" }})
        local issue = c.issues:create("octocat/hello-world", "New issue", "Description here", {{
            labels = {{"bug", "urgent"}},
        }})
        assert.eq(issue.number, 42)
        assert.eq(issue.title, "New issue")
        assert.eq(issue.state, "open")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_github_issue_comment() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/octocat/hello-world/issues/10/comments"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": 500,
            "body": "Looking into this now"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local github = require("assay.github")
        local c = github.client({{ token = "ghp_test", base_url = "{}" }})
        local comment = c.issues:create_note("octocat/hello-world", 10, "Looking into this now")
        assert.eq(comment.id, 500)
        assert.eq(comment.body, "Looking into this now")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_github_graphql() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "repository": {
                    "name": "hello-world",
                    "stargazerCount": 100
                }
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local github = require("assay.github")
        local c = github.client({{ token = "ghp_test", base_url = "{}" }})
        local result = c:graphql("query {{ repository(owner: \"octocat\", name: \"hello-world\") {{ name stargazerCount }} }}")
        assert.eq(result.data.repository.name, "hello-world")
        assert.eq(result.data.repository.stargazerCount, 100)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_github_repo_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octocat/hello-world"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "full_name": "octocat/hello-world",
            "description": "My first repository",
            "stargazers_count": 80,
            "language": "Lua",
            "default_branch": "main"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local github = require("assay.github")
        local c = github.client({{ base_url = "{}" }})
        local repo = c.repos:get("octocat/hello-world")
        assert.eq(repo.full_name, "octocat/hello-world")
        assert.eq(repo.stargazers_count, 80)
        assert.eq(repo.language, "Lua")
        assert.eq(repo.default_branch, "main")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_github_runs_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octocat/hello-world/actions/runs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 2,
            "workflow_runs": [
                { "id": 100, "name": "CI", "status": "completed", "conclusion": "success" },
                { "id": 101, "name": "CI", "status": "in_progress", "conclusion": null }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local github = require("assay.github")
        local c = github.client({{ base_url = "{}" }})
        local result = c.runs:list("octocat/hello-world")
        assert.eq(result.total_count, 2)
        assert.eq(#result.workflow_runs, 2)
        assert.eq(result.workflow_runs[1].id, 100)
        assert.eq(result.workflow_runs[1].conclusion, "success")
        assert.eq(result.workflow_runs[2].status, "in_progress")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_github_run_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octocat/hello-world/actions/runs/100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 100,
            "name": "CI",
            "status": "completed",
            "conclusion": "success"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local github = require("assay.github")
        local c = github.client({{ base_url = "{}" }})
        local run = c.runs:get("octocat/hello-world", 100)
        assert.eq(run.id, 100)
        assert.eq(run.status, "completed")
        assert.eq(run.conclusion, "success")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_github_pr_404_returns_nil() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octocat/hello-world/pulls/999"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "message": "Not Found"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local github = require("assay.github")
        local c = github.client({{ base_url = "{}" }})
        local pr = c.pulls:get("octocat/hello-world", 999)
        assert.eq(pr, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
