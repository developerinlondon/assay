mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_harbor_health() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2.0/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "healthy",
            "components": [
                {"name": "core", "status": "healthy"},
                {"name": "database", "status": "healthy"},
                {"name": "jobservice", "status": "healthy"},
                {"name": "redis", "status": "healthy"},
                {"name": "registry", "status": "healthy"},
                {"name": "registryctl", "status": "healthy"},
                {"name": "portal", "status": "healthy"}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local harbor = require("assay.harbor")
        local c = harbor.client("{}")
        local h = c:health()
        assert.eq(h.status, "healthy")
        assert.eq(#h.components, 7)
        assert.eq(h.components[1].name, "core")
        assert.eq(h.components[1].status, "healthy")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_harbor_is_healthy_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2.0/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "healthy",
            "components": [
                {"name": "core", "status": "healthy"},
                {"name": "database", "status": "healthy"},
                {"name": "redis", "status": "healthy"}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local harbor = require("assay.harbor")
        local c = harbor.client("{}")
        local healthy = c:is_healthy()
        assert.eq(healthy, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_harbor_is_healthy_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2.0/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "unhealthy",
            "components": [
                {"name": "core", "status": "healthy"},
                {"name": "database", "status": "unhealthy"},
                {"name": "redis", "status": "healthy"}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local harbor = require("assay.harbor")
        local c = harbor.client("{}")
        local healthy = c:is_healthy()
        assert.eq(healthy, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_harbor_system_info() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2.0/systeminfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "with_notary": false,
            "with_trivy": true,
            "auth_mode": "db_auth",
            "harbor_version": "v2.11.0-abc12345",
            "registry_url": "registry.example.com",
            "project_creation_restriction": "everyone",
            "self_registration": false,
            "has_ca_root": false,
            "registry_storage_provider_name": "s3"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local harbor = require("assay.harbor")
        local c = harbor.client("{}")
        local info = c:system_info()
        assert.eq(info.harbor_version, "v2.11.0-abc12345")
        assert.eq(info.auth_mode, "db_auth")
        assert.eq(info.with_trivy, true)
        assert.eq(info.registry_storage_provider_name, "s3")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_harbor_statistics() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2.0/statistics"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "private_project_count": 5,
            "private_repo_count": 23,
            "public_project_count": 2,
            "public_repo_count": 8,
            "total_project_count": 7,
            "total_repo_count": 31
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local harbor = require("assay.harbor")
        local c = harbor.client("{}")
        local stats = c:statistics()
        assert.eq(stats.total_project_count, 7)
        assert.eq(stats.total_repo_count, 31)
        assert.eq(stats.private_project_count, 5)
        assert.eq(stats.public_repo_count, 8)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_harbor_projects() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2.0/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "project_id": 1,
                "name": "library",
                "public": true,
                "repo_count": 12,
                "metadata": {"public": "true"},
                "creation_time": "2024-01-15T10:30:00.000Z",
                "update_time": "2024-06-20T14:00:00.000Z"
            },
            {
                "project_id": 2,
                "name": "my-app",
                "public": false,
                "repo_count": 5,
                "metadata": {"public": "false"},
                "creation_time": "2024-03-10T08:00:00.000Z",
                "update_time": "2024-07-01T12:00:00.000Z"
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local harbor = require("assay.harbor")
        local c = harbor.client("{}")
        local projects = c:projects()
        assert.eq(#projects, 2)
        assert.eq(projects[1].name, "library")
        assert.eq(projects[1].project_id, 1)
        assert.eq(projects[1].repo_count, 12)
        assert.eq(projects[2].name, "my-app")
        assert.eq(projects[2].public, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_harbor_project() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2.0/projects/library"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "project_id": 1,
            "name": "library",
            "public": true,
            "repo_count": 12,
            "metadata": {"public": "true", "auto_scan": "true"},
            "owner_name": "admin",
            "creation_time": "2024-01-15T10:30:00.000Z",
            "update_time": "2024-06-20T14:00:00.000Z",
            "cve_allowlist": {"items": [], "project_id": 1}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local harbor = require("assay.harbor")
        local c = harbor.client("{}")
        local proj = c:project("library")
        assert.eq(proj.project_id, 1)
        assert.eq(proj.name, "library")
        assert.eq(proj.owner_name, "admin")
        assert.eq(proj.repo_count, 12)
        assert.eq(proj.metadata.auto_scan, "true")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_harbor_repositories() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2.0/projects/library/repositories"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": 1,
                "name": "library/nginx",
                "project_id": 1,
                "artifact_count": 3,
                "pull_count": 150,
                "creation_time": "2024-02-01T09:00:00.000Z",
                "update_time": "2024-07-15T16:30:00.000Z"
            },
            {
                "id": 2,
                "name": "library/redis",
                "project_id": 1,
                "artifact_count": 2,
                "pull_count": 80,
                "creation_time": "2024-03-01T10:00:00.000Z",
                "update_time": "2024-07-10T11:00:00.000Z"
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local harbor = require("assay.harbor")
        local c = harbor.client("{}")
        local repos = c:repositories("library")
        assert.eq(#repos, 2)
        assert.eq(repos[1].name, "library/nginx")
        assert.eq(repos[1].artifact_count, 3)
        assert.eq(repos[1].pull_count, 150)
        assert.eq(repos[2].name, "library/redis")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_harbor_artifacts() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v2.0/projects/library/repositories/nginx/artifacts",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": 1,
                "digest": "sha256:aaaaaaaabbbbbbbbccccccccdddddddd",
                "size": 52428800,
                "push_time": "2024-07-15T16:30:00.000Z",
                "pull_time": "2024-07-16T08:00:00.000Z",
                "type": "IMAGE",
                "tags": [
                    {"name": "latest", "push_time": "2024-07-15T16:30:00.000Z"},
                    {"name": "1.25.0", "push_time": "2024-07-15T16:30:00.000Z"}
                ]
            },
            {
                "id": 2,
                "digest": "sha256:eeeeeeeefffffff0000000011111111",
                "size": 51380224,
                "push_time": "2024-07-10T12:00:00.000Z",
                "pull_time": "2024-07-14T20:00:00.000Z",
                "type": "IMAGE",
                "tags": [
                    {"name": "1.24.0", "push_time": "2024-07-10T12:00:00.000Z"}
                ]
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local harbor = require("assay.harbor")
        local c = harbor.client("{}")
        local arts = c:artifacts("library", "nginx")
        assert.eq(#arts, 2)
        assert.eq(arts[1].digest, "sha256:aaaaaaaabbbbbbbbccccccccdddddddd")
        assert.eq(arts[1].type, "IMAGE")
        assert.eq(#arts[1].tags, 2)
        assert.eq(arts[1].tags[1].name, "latest")
        assert.eq(arts[2].tags[1].name, "1.24.0")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_harbor_artifact() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v2.0/projects/library/repositories/nginx/artifacts/latest",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 1,
            "digest": "sha256:aaaaaaaabbbbbbbbccccccccdddddddd",
            "size": 52428800,
            "push_time": "2024-07-15T16:30:00.000Z",
            "type": "IMAGE",
            "tags": [
                {"name": "latest", "push_time": "2024-07-15T16:30:00.000Z"},
                {"name": "1.25.0", "push_time": "2024-07-15T16:30:00.000Z"}
            ],
            "extra_attrs": {
                "architecture": "amd64",
                "os": "linux"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local harbor = require("assay.harbor")
        local c = harbor.client("{}")
        local art = c:artifact("library", "nginx", "latest")
        assert.eq(art.digest, "sha256:aaaaaaaabbbbbbbbccccccccdddddddd")
        assert.eq(art.size, 52428800)
        assert.eq(art.type, "IMAGE")
        assert.eq(#art.tags, 2)
        assert.eq(art.extra_attrs.architecture, "amd64")
        assert.eq(art.extra_attrs.os, "linux")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_harbor_artifact_tags() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2.0/projects/library/repositories/nginx/artifacts/sha256:aaaaaaaabbbbbbbbccccccccdddddddd/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": 1,
                "name": "latest",
                "artifact_id": 1,
                "push_time": "2024-07-15T16:30:00.000Z",
                "immutable": false
            },
            {
                "id": 2,
                "name": "1.25.0",
                "artifact_id": 1,
                "push_time": "2024-07-15T16:30:00.000Z",
                "immutable": true
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local harbor = require("assay.harbor")
        local c = harbor.client("{}")
        local tags = c:artifact_tags("library", "nginx", "sha256:aaaaaaaabbbbbbbbccccccccdddddddd")
        assert.eq(#tags, 2)
        assert.eq(tags[1].name, "latest")
        assert.eq(tags[1].immutable, false)
        assert.eq(tags[2].name, "1.25.0")
        assert.eq(tags[2].immutable, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_harbor_image_exists_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v2.0/projects/library/repositories/nginx/artifacts/1.25.0",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 1,
            "digest": "sha256:aaaaaaaabbbbbbbbccccccccdddddddd",
            "type": "IMAGE"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local harbor = require("assay.harbor")
        local c = harbor.client("{}")
        local exists = c:image_exists("library", "nginx", "1.25.0")
        assert.eq(exists, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_harbor_image_exists_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2.0/projects/library/repositories/nginx/artifacts/99.99.99"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "errors": [{"code": "NOT_FOUND", "message": "artifact library/nginx:99.99.99 not found"}]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local harbor = require("assay.harbor")
        local c = harbor.client("{}")
        local exists = c:image_exists("library", "nginx", "99.99.99")
        assert.eq(exists, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_harbor_scan_artifact() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/api/v2.0/projects/library/repositories/nginx/artifacts/latest/scan",
        ))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local harbor = require("assay.harbor")
        local c = harbor.client("{}")
        local ok = c:scan_artifact("library", "nginx", "latest")
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_harbor_replication_policies() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2.0/replication/policies"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": 1,
                "name": "push-to-dockerhub",
                "src_registry": null,
                "dest_registry": {"id": 1, "name": "dockerhub"},
                "dest_namespace": "myorg",
                "trigger": {"type": "manual"},
                "enabled": true,
                "creation_time": "2024-01-20T10:00:00.000Z",
                "update_time": "2024-06-01T12:00:00.000Z"
            },
            {
                "id": 2,
                "name": "pull-from-gcr",
                "src_registry": {"id": 2, "name": "gcr"},
                "dest_registry": null,
                "trigger": {"type": "scheduled", "trigger_settings": {"cron": "0 0 * * *"}},
                "enabled": true,
                "creation_time": "2024-02-15T08:00:00.000Z",
                "update_time": "2024-05-20T09:00:00.000Z"
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local harbor = require("assay.harbor")
        local c = harbor.client("{}")
        local policies = c:replication_policies()
        assert.eq(#policies, 2)
        assert.eq(policies[1].name, "push-to-dockerhub")
        assert.eq(policies[1].enabled, true)
        assert.eq(policies[1].dest_namespace, "myorg")
        assert.eq(policies[2].name, "pull-from-gcr")
        assert.eq(policies[2].trigger.type, "scheduled")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_harbor_basic_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2.0/projects"))
        .and(wiremock::matchers::header(
            "Authorization",
            "Basic YWRtaW46SGFyYm9yMTIzNDU=",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"project_id": 1, "name": "library"}
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local harbor = require("assay.harbor")
        local c = harbor.client("{}", {{ username = "admin", password = "Harbor12345" }})
        local projects = c:projects()
        assert.eq(#projects, 1)
        assert.eq(projects[1].name, "library")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_harbor_robot_token_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2.0/projects"))
        .and(wiremock::matchers::header(
            "Authorization",
            "Bearer robot$mytoken123",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"project_id": 1, "name": "library"}
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local harbor = require("assay.harbor")
        local c = harbor.client("{}", {{ api_key = "robot$mytoken123" }})
        local projects = c:projects()
        assert.eq(#projects, 1)
        assert.eq(projects[1].name, "library")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
