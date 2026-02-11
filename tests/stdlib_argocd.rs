mod common;

use common::run_lua;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_argocd_client_creation() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/version"))
        .and(header("Authorization", "Bearer test-token-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "Version": "v2.10.0",
            "BuildDate": "2024-01-15T00:00:00Z",
            "GitCommit": "abc123"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}", {{ token = "test-token-123" }})
        local v = c:version()
        assert.eq(v.Version, "v2.10.0")
        assert.eq(v.GitCommit, "abc123")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_basic_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/version"))
        .and(header("Authorization", "Basic YWRtaW46cGFzc3dvcmQ="))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "Version": "v2.10.0"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}", {{ username = "admin", password = "password" }})
        local v = c:version()
        assert.eq(v.Version, "v2.10.0")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_applications_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/applications"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {},
            "items": [
                {
                    "metadata": {"name": "guestbook", "namespace": "argocd"},
                    "spec": {"project": "default", "source": {"repoURL": "https://github.com/example/repo"}},
                    "status": {
                        "health": {"status": "Healthy"},
                        "sync": {"status": "Synced"}
                    }
                },
                {
                    "metadata": {"name": "my-app", "namespace": "argocd"},
                    "spec": {"project": "production", "source": {"repoURL": "https://github.com/example/app"}},
                    "status": {
                        "health": {"status": "Degraded"},
                        "sync": {"status": "OutOfSync"}
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        local apps = c:applications()
        assert.eq(#apps, 2)
        assert.eq(apps[1].metadata.name, "guestbook")
        assert.eq(apps[1].status.health.status, "Healthy")
        assert.eq(apps[2].metadata.name, "my-app")
        assert.eq(apps[2].status.sync.status, "OutOfSync")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_application_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/applications/guestbook"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "guestbook", "namespace": "argocd", "uid": "abc-123"},
            "spec": {
                "project": "default",
                "source": {
                    "repoURL": "https://github.com/argoproj/argocd-example-apps",
                    "path": "guestbook",
                    "targetRevision": "HEAD"
                },
                "destination": {
                    "server": "https://kubernetes.default.svc",
                    "namespace": "default"
                }
            },
            "status": {
                "health": {"status": "Healthy", "message": "All resources healthy"},
                "sync": {"status": "Synced", "revision": "abc123def"}
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        local app = c:application("guestbook")
        assert.eq(app.metadata.name, "guestbook")
        assert.eq(app.metadata.uid, "abc-123")
        assert.eq(app.spec.source.path, "guestbook")
        assert.eq(app.status.health.status, "Healthy")
        assert.eq(app.status.sync.revision, "abc123def")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_app_health() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/applications/guestbook"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "guestbook"},
            "spec": {"project": "default"},
            "status": {
                "health": {"status": "Degraded", "message": "Pod crash loop"},
                "sync": {"status": "Synced"}
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        local h = c:app_health("guestbook")
        assert.eq(h.status, "Degraded")
        assert.eq(h.sync, "Synced")
        assert.eq(h.message, "Pod crash loop")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_sync() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/applications/guestbook/sync"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "guestbook"},
            "spec": {"project": "default"},
            "status": {
                "operationState": {
                    "phase": "Running",
                    "message": "Sync in progress",
                    "syncResult": {
                        "revision": "def456"
                    }
                }
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        local result = c:sync("guestbook", {{ revision = "def456", prune = true }})
        assert.eq(result.status.operationState.phase, "Running")
        assert.eq(result.status.operationState.syncResult.revision, "def456")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_refresh() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/applications/guestbook"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "guestbook"},
            "spec": {"project": "default"},
            "status": {
                "health": {"status": "Healthy"},
                "sync": {"status": "Synced", "revision": "latest123"}
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        local app = c:refresh("guestbook", {{ type = "hard" }})
        assert.eq(app.metadata.name, "guestbook")
        assert.eq(app.status.sync.revision, "latest123")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_rollback() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/applications/guestbook/rollback"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "guestbook"},
            "status": {
                "operationState": {
                    "phase": "Succeeded",
                    "message": "Rollback complete"
                }
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        local result = c:rollback("guestbook", 5)
        assert.eq(result.status.operationState.phase, "Succeeded")
        assert.eq(result.status.operationState.message, "Rollback complete")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_app_resources() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/applications/guestbook/resource-tree"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "nodes": [
                {
                    "group": "apps",
                    "version": "v1",
                    "kind": "Deployment",
                    "name": "guestbook-ui",
                    "namespace": "default",
                    "health": {"status": "Healthy"}
                },
                {
                    "group": "",
                    "version": "v1",
                    "kind": "Service",
                    "name": "guestbook-ui",
                    "namespace": "default",
                    "health": {"status": "Healthy"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        local tree = c:app_resources("guestbook")
        assert.eq(#tree.nodes, 2)
        assert.eq(tree.nodes[1].kind, "Deployment")
        assert.eq(tree.nodes[1].name, "guestbook-ui")
        assert.eq(tree.nodes[2].kind, "Service")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_app_manifests() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/applications/guestbook/manifests"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "manifests": [
                "{\"apiVersion\":\"v1\",\"kind\":\"Service\"}",
                "{\"apiVersion\":\"apps/v1\",\"kind\":\"Deployment\"}"
            ],
            "revision": "abc123"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        local result = c:app_manifests("guestbook")
        assert.eq(#result.manifests, 2)
        assert.eq(result.revision, "abc123")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_delete_app() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/applications/guestbook"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "guestbook"},
            "spec": {"project": "default"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        local result = c:delete_app("guestbook", {{ cascade = true, propagation_policy = "foreground" }})
        assert.eq(result.metadata.name, "guestbook")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_projects_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {},
            "items": [
                {
                    "metadata": {"name": "default"},
                    "spec": {
                        "description": "Default project",
                        "sourceRepos": ["*"],
                        "destinations": [{"server": "*", "namespace": "*"}]
                    }
                },
                {
                    "metadata": {"name": "production"},
                    "spec": {
                        "description": "Production apps",
                        "sourceRepos": ["https://github.com/myorg/*"],
                        "destinations": [{"server": "https://prod-cluster", "namespace": "prod"}]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        local projects = c:projects()
        assert.eq(#projects, 2)
        assert.eq(projects[1].metadata.name, "default")
        assert.eq(projects[1].spec.description, "Default project")
        assert.eq(projects[2].metadata.name, "production")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_project_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/default"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "default"},
            "spec": {
                "description": "Default project",
                "sourceRepos": ["*"],
                "destinations": [{"server": "*", "namespace": "*"}]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        local proj = c:project("default")
        assert.eq(proj.metadata.name, "default")
        assert.eq(proj.spec.description, "Default project")
        assert.eq(proj.spec.sourceRepos[1], "*")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_repositories_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repositories"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {},
            "items": [
                {
                    "repo": "https://github.com/argoproj/argocd-example-apps",
                    "type": "git",
                    "connectionState": {"status": "Successful"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        local repos = c:repositories()
        assert.eq(#repos, 1)
        assert.eq(repos[1].type, "git")
        assert.eq(repos[1].connectionState.status, "Successful")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_clusters_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/clusters"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {},
            "items": [
                {
                    "server": "https://kubernetes.default.svc",
                    "name": "in-cluster",
                    "connectionState": {"status": "Successful"},
                    "info": {"serverVersion": "1.28.0"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        local clusters = c:clusters()
        assert.eq(#clusters, 1)
        assert.eq(clusters[1].name, "in-cluster")
        assert.eq(clusters[1].connectionState.status, "Successful")
        assert.eq(clusters[1].info.serverVersion, "1.28.0")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_settings() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/settings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "url": "https://argocd.example.com",
            "dexConfig": {"connectors": []},
            "statusBadgeEnabled": true,
            "kustomizeVersions": ["v5.0.0"]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        local settings = c:settings()
        assert.eq(settings.url, "https://argocd.example.com")
        assert.eq(settings.statusBadgeEnabled, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_version() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/version"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "Version": "v2.10.0+abc123",
            "BuildDate": "2024-01-15T00:00:00Z",
            "GitCommit": "abc123def456",
            "GitTreeState": "clean",
            "GoVersion": "go1.21.5",
            "Compiler": "gc",
            "Platform": "linux/amd64"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        local v = c:version()
        assert.eq(v.Version, "v2.10.0+abc123")
        assert.eq(v.GitCommit, "abc123def456")
        assert.eq(v.Platform, "linux/amd64")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_is_healthy_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/applications/guestbook"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "guestbook"},
            "spec": {"project": "default"},
            "status": {
                "health": {"status": "Healthy"},
                "sync": {"status": "Synced"}
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        assert.eq(c:is_healthy("guestbook"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_is_healthy_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/applications/guestbook"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "guestbook"},
            "spec": {"project": "default"},
            "status": {
                "health": {"status": "Degraded", "message": "Container failing"},
                "sync": {"status": "Synced"}
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        assert.eq(c:is_healthy("guestbook"), false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_is_synced_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/applications/guestbook"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "guestbook"},
            "spec": {"project": "default"},
            "status": {
                "health": {"status": "Healthy"},
                "sync": {"status": "Synced"}
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        assert.eq(c:is_synced("guestbook"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_is_synced_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/applications/guestbook"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "guestbook"},
            "spec": {"project": "default"},
            "status": {
                "health": {"status": "Healthy"},
                "sync": {"status": "OutOfSync"}
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        assert.eq(c:is_synced("guestbook"), false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_error_on_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/applications/missing"))
        .respond_with(ResponseTemplate::new(404).set_body_string(r#"{"message":"application not found"}"#))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}")
        local ok, err = pcall(function() c:application("missing") end)
        assert.eq(ok, false)
        assert.matches(err, "HTTP 404")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_argocd_strip_trailing_slashes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/version"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "Version": "v2.10.0"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local argocd = require("assay.argocd")
        local c = argocd.client("{}///")
        local v = c:version()
        assert.eq(v.Version, "v2.10.0")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
