mod common;

use common::run_lua;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_gitlab() {
    let script = r#"
        local mod = require("assay.gitlab")
        assert.not_nil(mod)
        assert.not_nil(mod.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_sub_objects_exist() {
    let script = r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("http://localhost", { token = "test" })
        assert.not_nil(c.projects)
        assert.not_nil(c.files)
        assert.not_nil(c.commits)
        assert.not_nil(c.branches)
        assert.not_nil(c.tags)
        assert.not_nil(c.merge_requests)
        assert.not_nil(c.pipelines)
        assert.not_nil(c.jobs)
        assert.not_nil(c.releases)
        assert.not_nil(c.issues)
        assert.not_nil(c.groups)
        assert.not_nil(c.registry)
        assert.not_nil(c.hooks)
        assert.not_nil(c.users)
        assert.not_nil(c.environments)
        assert.not_nil(c.deploy_tokens)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_projects_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42"))
        .and(header("PRIVATE-TOKEN", "glpat-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 42,
            "name": "demo-project",
            "default_branch": "main",
            "web_url": "https://gitlab.example.com/demo/demo-project"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local p = c.projects:get(42)
        assert.eq(p.id, 42)
        assert.eq(p.name, "demo-project")
        assert.eq(p.default_branch, "main")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_projects_get_404_returns_nil() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/999"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "message": "404 Project Not Found"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local p = c.projects:get(999)
        assert.eq(p, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_files_raw() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/repository/files/src%2Fmain.lua/raw"))
        .and(query_param("ref", "dev"))
        .respond_with(ResponseTemplate::new(200).set_body_string("print(\"hello\")"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local content = c.files:raw(42, "src/main.lua", {{ ref = "dev" }})
        assert.eq(content, 'print("hello")')
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_commits_create() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v4/projects/42/repository/commits"))
        .and(header("PRIVATE-TOKEN", "glpat-test"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "abc123def456",
            "short_id": "abc123d",
            "title": "Update config files",
            "message": "Update config files",
            "author_name": "Automation"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local result = c.commits:create(42, {{
            branch = "main",
            commit_message = "Update config files",
            actions = {{
                {{ action = "update", file_path = "config.yaml", content = "key: value" }},
            }},
        }})
        assert.eq(result.short_id, "abc123d")
        assert.eq(result.title, "Update config files")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_commits_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/repository/commits/abc123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "abc123def456789",
            "short_id": "abc123d",
            "title": "Initial commit",
            "author_name": "Demo User"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local commit = c.commits:get(42, "abc123")
        assert.eq(commit.short_id, "abc123d")
        assert.eq(commit.title, "Initial commit")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_branches_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/repository/branches"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "name": "main", "default": true, "protected": true },
            { "name": "dev", "default": false, "protected": false }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local branches = c.branches:list(42)
        assert.eq(#branches, 2)
        assert.eq(branches[1].name, "main")
        assert.eq(branches[1].protected, true)
        assert.eq(branches[2].name, "dev")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_branches_create() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v4/projects/42/repository/branches"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "name": "feat/new-feature",
            "commit": { "id": "abc123", "short_id": "abc123" }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local b = c.branches:create(42, {{ branch = "feat/new-feature", ref = "main" }})
        assert.eq(b.name, "feat/new-feature")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_tags_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/repository/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "name": "v1.0.0", "message": "First release" },
            { "name": "v0.9.0", "message": "Beta" }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local tags = c.tags:list(42)
        assert.eq(#tags, 2)
        assert.eq(tags[1].name, "v1.0.0")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_merge_requests_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/merge_requests"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "iid": 1, "title": "Add feature", "state": "opened", "author": { "username": "alice" } },
            { "iid": 2, "title": "Fix bug", "state": "merged", "author": { "username": "bob" } }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local mrs = c.merge_requests:list(42)
        assert.eq(#mrs, 2)
        assert.eq(mrs[1].iid, 1)
        assert.eq(mrs[1].title, "Add feature")
        assert.eq(mrs[2].author.username, "bob")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_merge_requests_create() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v4/projects/42/merge_requests"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "iid": 10,
            "title": "New feature",
            "state": "opened",
            "web_url": "https://gitlab.example.com/demo/project/-/merge_requests/10"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local mr = c.merge_requests:create(42, {{
            source_branch = "feat/new-feature",
            target_branch = "main",
            title = "New feature",
        }})
        assert.eq(mr.iid, 10)
        assert.eq(mr.title, "New feature")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_merge_requests_merge() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v4/projects/42/merge_requests/10/merge"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "iid": 10,
            "state": "merged",
            "merge_commit_sha": "abc123def456"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local result = c.merge_requests:merge(42, 10, {{ squash = true }})
        assert.eq(result.state, "merged")
        assert.eq(result.merge_commit_sha, "abc123def456")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_pipelines_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/pipelines"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "id": 100, "status": "success", "ref": "main", "sha": "abc123" },
            { "id": 101, "status": "running", "ref": "dev", "sha": "def456" }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local pipes = c.pipelines:list(42)
        assert.eq(#pipes, 2)
        assert.eq(pipes[1].id, 100)
        assert.eq(pipes[1].status, "success")
        assert.eq(pipes[2].status, "running")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_pipelines_create() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v4/projects/42/pipeline"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": 200,
            "status": "pending",
            "ref": "main"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local pipe = c.pipelines:create(42, {{ ref = "main" }})
        assert.eq(pipe.id, 200)
        assert.eq(pipe.status, "pending")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_pipelines_jobs() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/pipelines/100/jobs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "id": 500, "name": "build", "status": "success", "stage": "build" },
            { "id": 501, "name": "test", "status": "failed", "stage": "test" }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local jobs = c.pipelines:jobs(42, 100)
        assert.eq(#jobs, 2)
        assert.eq(jobs[1].name, "build")
        assert.eq(jobs[2].status, "failed")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_issues_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "iid": 1, "title": "Bug report", "state": "opened", "labels": ["bug"] },
            { "iid": 2, "title": "Feature request", "state": "closed", "labels": ["enhancement"] }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local issues = c.issues:list(42)
        assert.eq(#issues, 2)
        assert.eq(issues[1].iid, 1)
        assert.eq(issues[1].labels[1], "bug")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_issues_create() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v4/projects/42/issues"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "iid": 5,
            "title": "New issue",
            "state": "opened"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local issue = c.issues:create(42, {{
            title = "New issue",
            description = "Something to track",
        }})
        assert.eq(issue.iid, 5)
        assert.eq(issue.state, "opened")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_releases_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "tag_name": "v1.0.0", "name": "Version 1.0.0" },
            { "tag_name": "v0.9.0", "name": "Version 0.9.0" }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local releases = c.releases:list(42)
        assert.eq(#releases, 2)
        assert.eq(releases[1].tag_name, "v1.0.0")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_groups_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/groups"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "id": 10, "name": "demo-group", "full_path": "demo-group" },
            { "id": 11, "name": "infra", "full_path": "demo-group/infra" }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local groups = c.groups:list()
        assert.eq(#groups, 2)
        assert.eq(groups[1].name, "demo-group")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_registry_repositories() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/registry/repositories"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "id": 1, "name": "", "path": "demo-group/demo-project" },
            { "id": 2, "name": "api", "path": "demo-group/demo-project/api" }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local repos = c.registry:repositories(42)
        assert.eq(#repos, 2)
        assert.eq(repos[2].name, "api")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_users_current() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/user"))
        .and(header("PRIVATE-TOKEN", "glpat-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 1,
            "username": "demo-user",
            "email": "user@example.com"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local user = c.users:current()
        assert.eq(user.username, "demo-user")
        assert.eq(user.email, "user@example.com")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_oauth_token_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/user"))
        .and(header("Authorization", "Bearer oauth-test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 1,
            "username": "oauth-user"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ oauth_token = "oauth-test-token" }})
        local user = c.users:current()
        assert.eq(user.username, "oauth-user")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_repository_compare() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/repository/compare"))
        .and(query_param("from", "main"))
        .and(query_param("to", "dev"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "commits": [
                { "short_id": "abc123", "title": "feat: add feature" }
            ],
            "diffs": [
                { "old_path": "README.md", "new_path": "README.md" }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local diff = c.repository:compare(42, "main", "dev")
        assert.eq(#diff.commits, 1)
        assert.eq(diff.commits[1].title, "feat: add feature")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_hooks_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/hooks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "id": 1, "url": "https://hooks.example.com/push", "push_events": true },
            { "id": 2, "url": "https://hooks.example.com/mr", "merge_requests_events": true }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "glpat-test" }})
        local hooks = c.hooks:list(42)
        assert.eq(#hooks, 2)
        assert.eq(hooks[1].push_events, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gitlab_error_propagation() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "message": "403 Forbidden"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local gitlab = require("assay.gitlab")
        local c = gitlab.client("{}", {{ token = "bad-token" }})
        local ok, err = pcall(function() c.projects:get(42) end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "HTTP 403")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
