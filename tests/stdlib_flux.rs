mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_flux() {
    let script = r#"
        local flux = require("assay.flux")
        assert.not_nil(flux)
        assert.not_nil(flux.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_git_repositories_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/source.toolkit.fluxcd.io/v1/namespaces/flux-system/gitrepositories",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "GitRepositoryList",
            "items": [
                {
                    "metadata": { "name": "app-repo", "namespace": "flux-system" },
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True"}],
                        "artifact": {"revision": "main@sha1:abc123"}
                    }
                },
                {
                    "metadata": { "name": "infra-repo", "namespace": "flux-system" },
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True"}]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local flux = require("assay.flux")
        local c = flux.client("{}", "fake-token")
        local repos = c:git_repositories("flux-system")
        assert.eq(repos.kind, "GitRepositoryList")
        assert.eq(#repos.items, 2)
        assert.eq(repos.items[1].metadata.name, "app-repo")
        assert.eq(repos.items[2].metadata.name, "infra-repo")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_git_repository_single() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/source.toolkit.fluxcd.io/v1/namespaces/flux-system/gitrepositories/app-repo",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "GitRepository",
            "metadata": { "name": "app-repo", "namespace": "flux-system" },
            "spec": { "url": "https://github.com/org/app", "ref": { "branch": "main" } },
            "status": {
                "conditions": [{"type": "Ready", "status": "True"}],
                "artifact": {"revision": "main@sha1:abc123"}
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local flux = require("assay.flux")
        local c = flux.client("{}", "fake-token")
        local repo = c:git_repository("flux-system", "app-repo")
        assert.eq(repo.kind, "GitRepository")
        assert.eq(repo.metadata.name, "app-repo")
        assert.eq(repo.spec.url, "https://github.com/org/app")
        assert.eq(repo.status.artifact.revision, "main@sha1:abc123")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_git_repo_ready_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/source.toolkit.fluxcd.io/v1/namespaces/flux-system/gitrepositories/app-repo",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": { "name": "app-repo" },
            "status": {
                "conditions": [
                    {"type": "Reconciling", "status": "False"},
                    {"type": "Ready", "status": "True"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local flux = require("assay.flux")
        local c = flux.client("{}", "fake-token")
        assert.eq(c:is_git_repo_ready("flux-system", "app-repo"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_git_repo_ready_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/source.toolkit.fluxcd.io/v1/namespaces/flux-system/gitrepositories/broken-repo",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": { "name": "broken-repo" },
            "status": {
                "conditions": [
                    {"type": "Ready", "status": "False", "reason": "GitOperationFailed"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local flux = require("assay.flux")
        local c = flux.client("{}", "fake-token")
        assert.eq(c:is_git_repo_ready("flux-system", "broken-repo"), false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_helm_repositories_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/source.toolkit.fluxcd.io/v1/namespaces/flux-system/helmrepositories",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "HelmRepositoryList",
            "items": [
                {
                    "metadata": { "name": "bitnami", "namespace": "flux-system" },
                    "spec": { "url": "https://charts.bitnami.com/bitnami" },
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True"}]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local flux = require("assay.flux")
        local c = flux.client("{}", "fake-token")
        local repos = c:helm_repositories("flux-system")
        assert.eq(repos.kind, "HelmRepositoryList")
        assert.eq(#repos.items, 1)
        assert.eq(repos.items[1].metadata.name, "bitnami")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kustomizations_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kustomize.toolkit.fluxcd.io/v1/namespaces/flux-system/kustomizations",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "KustomizationList",
            "items": [
                {
                    "metadata": { "name": "infra", "namespace": "flux-system" },
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True"}],
                        "lastAppliedRevision": "main@sha1:abc123",
                        "lastAttemptedRevision": "main@sha1:abc123"
                    }
                },
                {
                    "metadata": { "name": "apps", "namespace": "flux-system" },
                    "status": {
                        "conditions": [{"type": "Ready", "status": "False"}],
                        "lastAppliedRevision": "main@sha1:def456",
                        "lastAttemptedRevision": "main@sha1:ghi789"
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local flux = require("assay.flux")
        local c = flux.client("{}", "fake-token")
        local ks = c:kustomizations("flux-system")
        assert.eq(ks.kind, "KustomizationList")
        assert.eq(#ks.items, 2)
        assert.eq(ks.items[1].metadata.name, "infra")
        assert.eq(ks.items[2].metadata.name, "apps")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kustomization_single() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kustomize.toolkit.fluxcd.io/v1/namespaces/flux-system/kustomizations/infra",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "Kustomization",
            "metadata": { "name": "infra", "namespace": "flux-system" },
            "spec": {
                "sourceRef": { "kind": "GitRepository", "name": "app-repo" },
                "path": "./infrastructure"
            },
            "status": {
                "conditions": [{"type": "Ready", "status": "True"}],
                "lastAppliedRevision": "main@sha1:abc123",
                "lastAttemptedRevision": "main@sha1:abc123"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local flux = require("assay.flux")
        local c = flux.client("{}", "fake-token")
        local ks = c:kustomization("flux-system", "infra")
        assert.eq(ks.kind, "Kustomization")
        assert.eq(ks.metadata.name, "infra")
        assert.eq(ks.spec.path, "./infrastructure")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_kustomization_ready() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kustomize.toolkit.fluxcd.io/v1/namespaces/flux-system/kustomizations/infra",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": { "name": "infra" },
            "status": {
                "conditions": [{"type": "Ready", "status": "True"}]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local flux = require("assay.flux")
        local c = flux.client("{}", "fake-token")
        assert.eq(c:is_kustomization_ready("flux-system", "infra"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kustomization_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kustomize.toolkit.fluxcd.io/v1/namespaces/flux-system/kustomizations/infra",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": { "name": "infra" },
            "status": {
                "conditions": [
                    {"type": "Ready", "status": "True", "reason": "ReconciliationSucceeded"},
                    {"type": "Healthy", "status": "True"}
                ],
                "lastAppliedRevision": "main@sha1:abc123",
                "lastAttemptedRevision": "main@sha1:abc123"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local flux = require("assay.flux")
        local c = flux.client("{}", "fake-token")
        local st = c:kustomization_status("flux-system", "infra")
        assert.eq(st.ready, true)
        assert.eq(st.revision, "main@sha1:abc123")
        assert.eq(st.last_applied_revision, "main@sha1:abc123")
        assert.eq(#st.conditions, 2)
        assert.eq(st.conditions[1].reason, "ReconciliationSucceeded")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_helm_releases_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/helm.toolkit.fluxcd.io/v2/namespaces/default/helmreleases",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "HelmReleaseList",
            "items": [
                {
                    "metadata": { "name": "redis", "namespace": "default" },
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True"}]
                    }
                },
                {
                    "metadata": { "name": "postgres", "namespace": "default" },
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True"}]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local flux = require("assay.flux")
        local c = flux.client("{}", "fake-token")
        local releases = c:helm_releases("default")
        assert.eq(releases.kind, "HelmReleaseList")
        assert.eq(#releases.items, 2)
        assert.eq(releases.items[1].metadata.name, "redis")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_helm_release_ready() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/helm.toolkit.fluxcd.io/v2/namespaces/default/helmreleases/redis",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": { "name": "redis" },
            "status": {
                "conditions": [
                    {"type": "Released", "status": "True"},
                    {"type": "Ready", "status": "True"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local flux = require("assay.flux")
        local c = flux.client("{}", "fake-token")
        assert.eq(c:is_helm_release_ready("default", "redis"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_all_sources_ready() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path(
            "/apis/source.toolkit.fluxcd.io/v1/namespaces/flux-system/gitrepositories",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [
                {
                    "metadata": { "name": "app-repo" },
                    "status": { "conditions": [{"type": "Ready", "status": "True"}] }
                },
                {
                    "metadata": { "name": "broken-repo" },
                    "status": { "conditions": [{"type": "Ready", "status": "False"}] }
                }
            ]
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(
            "/apis/source.toolkit.fluxcd.io/v1/namespaces/flux-system/helmrepositories",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [
                {
                    "metadata": { "name": "bitnami" },
                    "status": { "conditions": [{"type": "Ready", "status": "True"}] }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local flux = require("assay.flux")
        local c = flux.client("{}", "fake-token")
        local result = c:all_sources_ready("flux-system")
        assert.eq(result.total, 3)
        assert.eq(result.ready, 2)
        assert.eq(result.not_ready, 1)
        assert.eq(result.not_ready_names[1], "broken-repo")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_all_kustomizations_ready() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path(
            "/apis/kustomize.toolkit.fluxcd.io/v1/namespaces/flux-system/kustomizations",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [
                {
                    "metadata": { "name": "infra" },
                    "status": { "conditions": [{"type": "Ready", "status": "True"}] }
                },
                {
                    "metadata": { "name": "apps" },
                    "status": { "conditions": [{"type": "Ready", "status": "True"}] }
                },
                {
                    "metadata": { "name": "monitoring" },
                    "status": { "conditions": [{"type": "Ready", "status": "False"}] }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local flux = require("assay.flux")
        local c = flux.client("{}", "fake-token")
        local result = c:all_kustomizations_ready("flux-system")
        assert.eq(result.total, 3)
        assert.eq(result.ready, 2)
        assert.eq(result.not_ready, 1)
        assert.eq(result.not_ready_names[1], "monitoring")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
