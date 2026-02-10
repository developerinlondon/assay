mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_k8s() {
    let script = r#"
        local k8s = require("assay.k8s")
        assert.not_nil(k8s)
        assert.not_nil(k8s.get)
        assert.not_nil(k8s.get_secret)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_get_with_base_url() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/default/pods"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "kind": "PodList",
                "items": [{"metadata": {"name": "test-pod"}}]
            })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local pods = k8s.get("/api/v1/namespaces/default/pods", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(pods.kind, "PodList")
        assert.eq(pods.items[1].metadata.name, "test-pod")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_get_secret() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/infra/secrets/db-creds"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "kind": "Secret",
                "data": {
                    "username": "YWRtaW4=",
                    "password": "c2VjcmV0"
                }
            })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local secret = k8s.get_secret("infra", "db-creds", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(secret.username, "admin")
        assert.eq(secret.password, "secret")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_exists_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/infra/secrets/db-creds"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local found = k8s.exists("infra", "secret", "db-creds", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(found, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_exists_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/infra/secrets/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local found = k8s.exists("infra", "secret", "missing", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(found, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_is_ready_deployment() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/apps/v1/namespaces/infra/deployments/api"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": {"replicas": 3, "readyReplicas": 3}
            })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local ready = k8s.is_ready("infra", "deployment", "api", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(ready, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_is_ready_deployment_not_ready() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/apps/v1/namespaces/infra/deployments/api"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": {"replicas": 3, "readyReplicas": 1}
            })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local ready = k8s.is_ready("infra", "deployment", "api", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(ready, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_is_ready_pod() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/infra/pods/worker-0"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": {
                    "conditions": [
                        {"type": "Ready", "status": "True"}
                    ]
                }
            })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local ready = k8s.is_ready("infra", "pod", "worker-0", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(ready, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_pod_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/infra/pods"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {"status": {"phase": "Running"}},
                    {"status": {"phase": "Running"}},
                    {"status": {"phase": "Pending"}},
                ]
            })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local status = k8s.pod_status("infra", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(status.total, 3)
        assert.eq(status.running, 2)
        assert.eq(status.pending, 1)
        assert.eq(status.failed, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_service_endpoints() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/infra/endpoints/postgres"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "subsets": [{
                    "addresses": [
                        {"ip": "10.42.0.5"},
                        {"ip": "10.42.0.6"}
                    ]
                }]
            })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local ips = k8s.service_endpoints("infra", "postgres", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(#ips, 2)
        assert.eq(ips[1], "10.42.0.5")
        assert.eq(ips[2], "10.42.0.6")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_service_endpoints_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/infra/endpoints/broken"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"subsets": []})),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local ips = k8s.service_endpoints("infra", "broken", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(#ips, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_logs() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/infra/pods/api-7b9d4/log"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("2026-02-10 INFO started\n2026-02-10 INFO ready\n"),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local output = k8s.logs("infra", "api-7b9d4", {{
            base_url = "{}",
            token = "fake-token",
            tail = 50,
        }})
        assert.contains(output, "started")
        assert.contains(output, "ready")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_rollout_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/apps/v1/namespaces/infra/deployments/api"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "spec": {"replicas": 3},
                "status": {
                    "updatedReplicas": 3,
                    "readyReplicas": 3,
                    "availableReplicas": 3,
                    "unavailableReplicas": 0,
                }
            })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local rs = k8s.rollout_status("infra", "api", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(rs.desired, 3)
        assert.eq(rs.ready, 3)
        assert.eq(rs.complete, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_register_crd() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/argoproj.io/v1alpha1/namespaces/argocd/applications/traefik",
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "kind": "Application",
                "metadata": {"name": "traefik"},
                "status": {"health": {"status": "Healthy"}, "sync": {"status": "Synced"}}
            })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        k8s.register_crd("application", "argoproj.io", "v1alpha1", "applications")
        local app = k8s.get_resource("argocd", "application", "traefik", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(app.metadata.name, "traefik")
        assert.eq(app.status.health.status, "Healthy")
        assert.eq(app.status.sync.status, "Synced")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_list_generic() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/apps/v1/namespaces/infra/deployments"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "kind": "DeploymentList",
                "items": [
                    {"metadata": {"name": "api"}},
                    {"metadata": {"name": "worker"}},
                ]
            })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local deploys = k8s.list("infra", "deployment", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(#deploys.items, 2)
        assert.eq(deploys.items[1].metadata.name, "api")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_is_ready_generic_conditions() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/networking.k8s.io/v1/namespaces/infra/ingresses/web"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": {
                    "conditions": [{"type": "Ready", "status": "True"}]
                }
            })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local ready = k8s.is_ready("infra", "ingress", "web", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(ready, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_is_ready_phase_fallback() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/infra/persistentvolumeclaims/data-vol"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"status": {"phase": "Bound"}})),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local ready = k8s.is_ready("infra", "pvc", "data-vol", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(ready, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_is_ready_job() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/batch/v1/namespaces/infra/jobs/migrate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"status": {"succeeded": 1}})),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local ready = k8s.is_ready("infra", "job", "migrate", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(ready, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_node_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/nodes"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{
                    "metadata": {
                        "name": "node-1",
                        "labels": {"node-role.kubernetes.io/control-plane": ""}
                    },
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True"}],
                        "capacity": {"cpu": "4", "memory": "8Gi"},
                        "allocatable": {"cpu": "3800m", "memory": "7Gi"}
                    }
                }]
            })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local nodes = k8s.node_status({{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(#nodes, 1)
        assert.eq(nodes[1].name, "node-1")
        assert.eq(nodes[1].ready, true)
        assert.eq(nodes[1].roles[1], "control-plane")
        assert.eq(nodes[1].capacity.cpu, "4")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_events_for() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/infra/events"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {"reason": "Scheduled", "message": "Successfully assigned pod"},
                    {"reason": "Pulled", "message": "Container image pulled"},
                ]
            })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local events = k8s.events_for("infra", "Pod", "api-7b9d4", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(#events.items, 2)
        assert.eq(events.items[1].reason, "Scheduled")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_k8s_get_configmap() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/infra/configmaps/gitops-config"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "kind": "ConfigMap",
                "data": {
                    "clusterDomain": "jeebon.xyz",
                    "environment": "test"
                }
            })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local cm = k8s.get_configmap("infra", "gitops-config", {{
            base_url = "{}",
            token = "fake-token",
        }})
        assert.eq(cm.clusterDomain, "jeebon.xyz")
        assert.eq(cm.environment, "test")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
