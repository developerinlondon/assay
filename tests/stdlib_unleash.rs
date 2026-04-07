mod common;

use common::run_lua;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_unleash() {
    let script = r#"
        local unleash = require("assay.unleash")
        assert.not_nil(unleash)
        assert.not_nil(unleash.client)
        assert.not_nil(unleash.wait)
        assert.not_nil(unleash.ensure_project)
        assert.not_nil(unleash.ensure_environment)
        assert.not_nil(unleash.ensure_token)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_health() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .and(header("Authorization", "*:*.test-admin-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "health": "GOOD"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "*:*.test-admin-token" }})
        local h = c:health()
        assert.eq(h.health, "GOOD")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_projects() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "version": 1,
            "projects": [
                {"id": "default", "name": "Default", "description": "Default project", "memberCount": 1, "featureCount": 5},
                {"id": "demo-project", "name": "Demo Project", "description": "Demo project description", "memberCount": 2, "featureCount": 3}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local projects = c:projects()
        assert.eq(#projects, 2)
        assert.eq(projects[1].id, "default")
        assert.eq(projects[2].id, "demo-project")
        assert.eq(projects[2].name, "Demo Project")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_project() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/projects/demo-project"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "demo-project",
            "name": "Demo Project",
            "description": "Demo project description",
            "environments": [
                {"environment": "development", "enabled": true},
                {"environment": "production", "enabled": true}
            ],
            "features": []
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local p = c:project("demo-project")
        assert.eq(p.id, "demo-project")
        assert.eq(p.name, "Demo Project")
        assert.eq(#p.environments, 2)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_project_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/projects/nonexistent"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local p = c:project("nonexistent")
        assert.eq(p, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_create_project() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/admin/projects"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "demo-project",
            "name": "Demo Project",
            "description": "Demo project description"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local p = c:create_project({{ id = "demo-project", name = "Demo Project", description = "Demo project description" }})
        assert.eq(p.id, "demo-project")
        assert.eq(p.name, "Demo Project")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_update_project() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/admin/projects/demo-project"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "demo-project",
            "name": "Demo Project Updated",
            "description": "Updated description"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local p = c:update_project("demo-project", {{ name = "Demo Project Updated", description = "Updated description" }})
        assert.eq(p.name, "Demo Project Updated")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_delete_project() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/admin/projects/demo-project"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        c:delete_project("demo-project")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_environments() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "version": 1,
            "environments": [
                {"name": "development", "type": "development", "enabled": true, "sortOrder": 1},
                {"name": "production", "type": "production", "enabled": true, "sortOrder": 2}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local envs = c:environments()
        assert.eq(#envs, 2)
        assert.eq(envs[1].name, "development")
        assert.eq(envs[2].name, "production")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_enable_environment() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/admin/projects/demo-project/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        c:enable_environment("demo-project", "production")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_disable_environment() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/admin/projects/demo-project/environments/staging"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        c:disable_environment("demo-project", "staging")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_features() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/projects/demo-project/features"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "version": 2,
            "features": [
                {"name": "dark-mode", "type": "release", "enabled": false, "project": "demo-project"},
                {"name": "new-dashboard", "type": "experiment", "enabled": true, "project": "demo-project"}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local features = c:features("demo-project")
        assert.eq(#features, 2)
        assert.eq(features[1].name, "dark-mode")
        assert.eq(features[2].name, "new-dashboard")
        assert.eq(features[2].enabled, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_feature() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/projects/demo-project/features/dark-mode"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "dark-mode",
            "type": "release",
            "project": "demo-project",
            "enabled": false,
            "environments": [
                {"name": "development", "enabled": true},
                {"name": "production", "enabled": false}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local f = c:feature("demo-project", "dark-mode")
        assert.eq(f.name, "dark-mode")
        assert.eq(f.type, "release")
        assert.eq(#f.environments, 2)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_feature_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/projects/demo-project/features/nonexistent"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local f = c:feature("demo-project", "nonexistent")
        assert.eq(f, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_create_feature() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/admin/projects/demo-project/features"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "name": "dark-mode",
            "type": "release",
            "project": "demo-project",
            "description": "Enable dark mode UI"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local f = c:create_feature("demo-project", {{ name = "dark-mode", type = "release", description = "Enable dark mode UI" }})
        assert.eq(f.name, "dark-mode")
        assert.eq(f.type, "release")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_update_feature() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/admin/projects/demo-project/features/dark-mode"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "dark-mode",
            "type": "release",
            "description": "Updated dark mode"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local f = c:update_feature("demo-project", "dark-mode", {{ description = "Updated dark mode" }})
        assert.eq(f.description, "Updated dark mode")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_archive_feature() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/admin/projects/demo-project/features/dark-mode"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        c:archive_feature("demo-project", "dark-mode")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_toggle_on() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/api/admin/projects/demo-project/features/dark-mode/environments/development/on",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        c:toggle_on("demo-project", "dark-mode", "development")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_toggle_off() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/api/admin/projects/demo-project/features/dark-mode/environments/production/off",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        c:toggle_off("demo-project", "dark-mode", "production")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_strategies() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/api/admin/projects/demo-project/features/dark-mode/environments/development/strategies",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"id": "strategy-1", "name": "default", "parameters": {}},
            {"id": "strategy-2", "name": "userWithId", "parameters": {"userIds": "user1,user2"}}
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local strats = c:strategies("demo-project", "dark-mode", "development")
        assert.eq(#strats, 2)
        assert.eq(strats[1].name, "default")
        assert.eq(strats[2].name, "userWithId")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_add_strategy() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/api/admin/projects/demo-project/features/dark-mode/environments/development/strategies",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "strategy-3",
            "name": "flexibleRollout",
            "parameters": {"rollout": "50", "stickiness": "default"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local s = c:add_strategy("demo-project", "dark-mode", "development", {{
            name = "flexibleRollout",
            parameters = {{ rollout = "50", stickiness = "default" }}
        }})
        assert.eq(s.name, "flexibleRollout")
        assert.eq(s.id, "strategy-3")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/api-tokens"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "tokens": [
                {"secret": "*:development.abc123", "tokenName": "demo-project-dev", "type": "client", "environment": "development", "projects": ["demo-project"]},
                {"secret": "*:*.admin456", "tokenName": "admin", "type": "admin", "projects": ["*"]}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local tokens = c:tokens()
        assert.eq(#tokens, 2)
        assert.eq(tokens[1].tokenName, "demo-project-dev")
        assert.eq(tokens[1].type, "client")
        assert.eq(tokens[2].type, "admin")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_create_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/admin/api-tokens"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "secret": "demo-project:development.newtoken789",
            "tokenName": "demo-project-client",
            "type": "client",
            "environment": "development",
            "projects": ["demo-project"],
            "createdAt": "2026-02-20T00:00:00Z"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local t = c:create_token({{
            tokenName = "demo-project-client",
            type = "client",
            environment = "development",
            projects = {{ "demo-project" }}
        }})
        assert.eq(t.secret, "demo-project:development.newtoken789")
        assert.eq(t.tokenName, "demo-project-client")
        assert.eq(t.type, "client")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_delete_token() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/admin/api-tokens/old-token-secret"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        c:delete_token("old-token-secret")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_wait_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"health": "GOOD"})),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local result = unleash.wait("{}", {{ timeout = 5, interval = 0.1 }})
        assert.eq(result, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_wait_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        unleash.wait("{}", {{ timeout = 1, interval = 0.5 }})
        "#,
        server.uri()
    );
    let result = run_lua(&script).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_unleash_ensure_project_existing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/projects/demo-project"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "demo-project",
            "name": "Demo Project",
            "description": "Existing project"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local p = unleash.ensure_project(c, "demo-project")
        assert.eq(p.id, "demo-project")
        assert.eq(p.name, "Demo Project")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_ensure_project_new() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/projects/new-project"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/admin/projects"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "new-project",
            "name": "New Project",
            "description": "A new project"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local p = unleash.ensure_project(c, "new-project", {{ name = "New Project", description = "A new project" }})
        assert.not_nil(p)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_ensure_environment_new() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/admin/projects/demo-project/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local result = unleash.ensure_environment(c, "demo-project", "qa")
        assert.eq(result, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_ensure_environment_already_exists() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/admin/projects/demo-project/environments"))
        .respond_with(ResponseTemplate::new(409).set_body_string("Environment already enabled"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local result = unleash.ensure_environment(c, "demo-project", "development")
        assert.eq(result, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_ensure_token_existing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/api-tokens"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "tokens": [
                {"secret": "hidden", "tokenName": "demo-project-dev", "type": "client", "environment": "development", "projects": ["demo-project"]}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local t = unleash.ensure_token(c, {{
            tokenName = "demo-project-dev",
            type = "client",
            environment = "development"
        }})
        assert.eq(t.tokenName, "demo-project-dev")
        assert.eq(t.type, "client")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unleash_ensure_token_new() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/api-tokens"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "tokens": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/admin/api-tokens"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "secret": "demo-project:production.newtoken",
            "tokenName": "demo-project-prod",
            "type": "client",
            "environment": "production",
            "projects": ["demo-project"]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local unleash = require("assay.unleash")
        local c = unleash.client("{}", {{ token = "test-token" }})
        local t = unleash.ensure_token(c, {{
            tokenName = "demo-project-prod",
            type = "client",
            environment = "production",
            projects = {{ "demo-project" }}
        }})
        assert.eq(t.secret, "demo-project:production.newtoken")
        assert.eq(t.tokenName, "demo-project-prod")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
