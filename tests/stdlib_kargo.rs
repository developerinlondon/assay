mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_kargo() {
    let script = r#"
        local kargo = require("assay.kargo")
        assert.not_nil(kargo)
        assert.not_nil(kargo.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_stages_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kargo.akuity.io/v1alpha1/namespaces/jeebon-test/stages",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "StageList",
            "items": [
                {
                    "apiVersion": "kargo.akuity.io/v1alpha1",
                    "kind": "Stage",
                    "metadata": {"name": "test", "namespace": "jeebon-test"},
                    "status": {"phase": "Steady"}
                },
                {
                    "apiVersion": "kargo.akuity.io/v1alpha1",
                    "kind": "Stage",
                    "metadata": {"name": "dev", "namespace": "jeebon-test"},
                    "status": {"phase": "Promoting"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}", "fake-token")
        local stages = c:stages("jeebon-test")
        assert.eq(#stages, 2)
        assert.eq(stages[1].metadata.name, "test")
        assert.eq(stages[1].status.phase, "Steady")
        assert.eq(stages[2].metadata.name, "dev")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_stage_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kargo.akuity.io/v1alpha1/namespaces/jeebon-test/stages/test",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "Stage",
            "metadata": {"name": "test", "namespace": "jeebon-test"},
            "spec": {
                "subscriptions": {"warehouse": "main"},
                "promotionMechanisms": {"argoCDAppUpdates": [{"appName": "jeebon-test"}]}
            },
            "status": {
                "phase": "Steady",
                "currentFreightId": "freight-abc123",
                "health": {"status": "Healthy"},
                "conditions": [
                    {"type": "Healthy", "status": "True", "reason": "AllHealthy"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}", "fake-token")
        local s = c:stage("jeebon-test", "test")
        assert.eq(s.metadata.name, "test")
        assert.eq(s.status.phase, "Steady")
        assert.eq(s.status.currentFreightId, "freight-abc123")
        assert.eq(s.status.health.status, "Healthy")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_stage_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kargo.akuity.io/v1alpha1/namespaces/jeebon-test/stages/test",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "Stage",
            "metadata": {"name": "test", "namespace": "jeebon-test"},
            "status": {
                "phase": "Steady",
                "currentFreightId": "freight-abc123",
                "health": {"status": "Healthy"},
                "conditions": [
                    {"type": "Healthy", "status": "True", "reason": "AllHealthy"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}", "fake-token")
        local st = c:stage_status("jeebon-test", "test")
        assert.eq(st.phase, "Steady")
        assert.eq(st.current_freight_id, "freight-abc123")
        assert.eq(st.health.status, "Healthy")
        assert.eq(#st.conditions, 1)
        assert.eq(st.conditions[1].type, "Healthy")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_is_stage_healthy_steady() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kargo.akuity.io/v1alpha1/namespaces/jeebon-test/stages/test",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "Stage",
            "metadata": {"name": "test", "namespace": "jeebon-test"},
            "status": {"phase": "Steady"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}", "fake-token")
        local healthy = c:is_stage_healthy("jeebon-test", "test")
        assert.eq(healthy, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_is_stage_healthy_condition() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kargo.akuity.io/v1alpha1/namespaces/jeebon-test/stages/test",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "Stage",
            "metadata": {"name": "test", "namespace": "jeebon-test"},
            "status": {
                "phase": "Promoting",
                "conditions": [
                    {"type": "Healthy", "status": "True", "reason": "AllHealthy"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}", "fake-token")
        local healthy = c:is_stage_healthy("jeebon-test", "test")
        assert.eq(healthy, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_is_stage_unhealthy() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kargo.akuity.io/v1alpha1/namespaces/jeebon-test/stages/broken",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "Stage",
            "metadata": {"name": "broken", "namespace": "jeebon-test"},
            "status": {
                "phase": "Failed",
                "conditions": [
                    {"type": "Healthy", "status": "False", "reason": "PromotionFailed"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}", "fake-token")
        local healthy = c:is_stage_healthy("jeebon-test", "broken")
        assert.eq(healthy, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_freight_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kargo.akuity.io/v1alpha1/namespaces/jeebon-test/freight",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "FreightList",
            "items": [
                {
                    "apiVersion": "kargo.akuity.io/v1alpha1",
                    "kind": "Freight",
                    "metadata": {"name": "freight-abc123", "namespace": "jeebon-test"},
                    "status": {
                        "verifiedIn": {"test": {}},
                        "approvedFor": {"dev": {}}
                    }
                },
                {
                    "apiVersion": "kargo.akuity.io/v1alpha1",
                    "kind": "Freight",
                    "metadata": {"name": "freight-def456", "namespace": "jeebon-test"},
                    "status": {
                        "verifiedIn": {},
                        "approvedFor": {}
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}", "fake-token")
        local freight = c:freight_list("jeebon-test")
        assert.eq(#freight, 2)
        assert.eq(freight[1].metadata.name, "freight-abc123")
        assert.eq(freight[2].metadata.name, "freight-def456")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_freight_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kargo.akuity.io/v1alpha1/namespaces/jeebon-test/freight/freight-abc123",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "Freight",
            "metadata": {"name": "freight-abc123", "namespace": "jeebon-test"},
            "spec": {
                "commits": [{"repoURL": "https://github.com/org/repo", "id": "abc123"}],
                "images": [{"repoURL": "ghcr.io/org/app", "tag": "v1.2.3"}]
            },
            "status": {
                "verifiedIn": {"test": {}},
                "approvedFor": {"dev": {}}
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}", "fake-token")
        local f = c:freight("jeebon-test", "freight-abc123")
        assert.eq(f.metadata.name, "freight-abc123")
        assert.eq(f.spec.images[1].tag, "v1.2.3")
        assert.eq(f.spec.commits[1].id, "abc123")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_freight_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kargo.akuity.io/v1alpha1/namespaces/jeebon-test/freight/freight-abc123",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "Freight",
            "metadata": {"name": "freight-abc123", "namespace": "jeebon-test"},
            "status": {
                "verifiedIn": {"test": {}},
                "approvedFor": {"dev": {}}
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}", "fake-token")
        local st = c:freight_status("jeebon-test", "freight-abc123")
        assert.not_nil(st.verifiedIn)
        assert.not_nil(st.approvedFor)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_promotions_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kargo.akuity.io/v1alpha1/namespaces/jeebon-test/promotions",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "PromotionList",
            "items": [
                {
                    "apiVersion": "kargo.akuity.io/v1alpha1",
                    "kind": "Promotion",
                    "metadata": {"name": "test-abc", "namespace": "jeebon-test"},
                    "spec": {"stage": "test", "freight": "freight-abc123"},
                    "status": {"phase": "Succeeded"}
                },
                {
                    "apiVersion": "kargo.akuity.io/v1alpha1",
                    "kind": "Promotion",
                    "metadata": {"name": "test-def", "namespace": "jeebon-test"},
                    "spec": {"stage": "test", "freight": "freight-def456"},
                    "status": {"phase": "Running"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}", "fake-token")
        local promos = c:promotions("jeebon-test")
        assert.eq(#promos, 2)
        assert.eq(promos[1].metadata.name, "test-abc")
        assert.eq(promos[1].status.phase, "Succeeded")
        assert.eq(promos[2].status.phase, "Running")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_promote() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/apis/kargo.akuity.io/v1alpha1/namespaces/jeebon-test/promotions",
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "Promotion",
            "metadata": {
                "name": "test-x7k9z",
                "namespace": "jeebon-test",
                "generateName": "test-"
            },
            "spec": {
                "stage": "test",
                "freight": "freight-abc123"
            },
            "status": {
                "phase": "Pending"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}", "fake-token")
        local promo = c:promote("jeebon-test", "test", "freight-abc123")
        assert.eq(promo.metadata.name, "test-x7k9z")
        assert.eq(promo.spec.stage, "test")
        assert.eq(promo.spec.freight, "freight-abc123")
        assert.eq(promo.status.phase, "Pending")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_promotion_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kargo.akuity.io/v1alpha1/namespaces/jeebon-test/promotions/test-abc",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "Promotion",
            "metadata": {"name": "test-abc", "namespace": "jeebon-test"},
            "spec": {"stage": "test", "freight": "freight-abc123"},
            "status": {
                "phase": "Succeeded",
                "message": "Promotion completed successfully",
                "freightId": "freight-abc123"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}", "fake-token")
        local st = c:promotion_status("jeebon-test", "test-abc")
        assert.eq(st.phase, "Succeeded")
        assert.eq(st.message, "Promotion completed successfully")
        assert.eq(st.freight_id, "freight-abc123")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_warehouses_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kargo.akuity.io/v1alpha1/namespaces/jeebon-test/warehouses",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "WarehouseList",
            "items": [
                {
                    "apiVersion": "kargo.akuity.io/v1alpha1",
                    "kind": "Warehouse",
                    "metadata": {"name": "main", "namespace": "jeebon-test"},
                    "spec": {
                        "subscriptions": [
                            {"image": {"repoURL": "ghcr.io/org/app", "semverConstraint": ">=1.0.0"}}
                        ]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}", "fake-token")
        local wh = c:warehouses("jeebon-test")
        assert.eq(#wh, 1)
        assert.eq(wh[1].metadata.name, "main")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_warehouse_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kargo.akuity.io/v1alpha1/namespaces/jeebon-test/warehouses/main",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "Warehouse",
            "metadata": {"name": "main", "namespace": "jeebon-test"},
            "spec": {
                "subscriptions": [
                    {"image": {"repoURL": "ghcr.io/org/app", "semverConstraint": ">=1.0.0"}}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}", "fake-token")
        local wh = c:warehouse("jeebon-test", "main")
        assert.eq(wh.metadata.name, "main")
        assert.eq(wh.kind, "Warehouse")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_projects_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/kargo.akuity.io/v1alpha1/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "ProjectList",
            "items": [
                {
                    "apiVersion": "kargo.akuity.io/v1alpha1",
                    "kind": "Project",
                    "metadata": {"name": "jeebon-test"},
                    "status": {"phase": "Ready"}
                },
                {
                    "apiVersion": "kargo.akuity.io/v1alpha1",
                    "kind": "Project",
                    "metadata": {"name": "jeebon-dev"},
                    "status": {"phase": "Ready"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}", "fake-token")
        local projects = c:projects()
        assert.eq(#projects, 2)
        assert.eq(projects[1].metadata.name, "jeebon-test")
        assert.eq(projects[2].metadata.name, "jeebon-dev")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_project_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/kargo.akuity.io/v1alpha1/projects/jeebon-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "Project",
            "metadata": {"name": "jeebon-test"},
            "status": {"phase": "Ready"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}", "fake-token")
        local p = c:project("jeebon-test")
        assert.eq(p.metadata.name, "jeebon-test")
        assert.eq(p.status.phase, "Ready")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_pipeline_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/kargo.akuity.io/v1alpha1/namespaces/jeebon-test/stages",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "StageList",
            "items": [
                {
                    "apiVersion": "kargo.akuity.io/v1alpha1",
                    "kind": "Stage",
                    "metadata": {"name": "test", "namespace": "jeebon-test"},
                    "status": {
                        "phase": "Steady",
                        "currentFreightId": "freight-abc123"
                    }
                },
                {
                    "apiVersion": "kargo.akuity.io/v1alpha1",
                    "kind": "Stage",
                    "metadata": {"name": "dev", "namespace": "jeebon-test"},
                    "status": {
                        "phase": "Promoting",
                        "currentFreightId": "freight-def456",
                        "conditions": [
                            {"type": "Healthy", "status": "True", "reason": "AllHealthy"}
                        ]
                    }
                },
                {
                    "apiVersion": "kargo.akuity.io/v1alpha1",
                    "kind": "Stage",
                    "metadata": {"name": "prod", "namespace": "jeebon-test"},
                    "status": {
                        "phase": "Failed",
                        "currentFreightId": "freight-old789",
                        "conditions": [
                            {"type": "Healthy", "status": "False", "reason": "PromotionFailed"}
                        ]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}", "fake-token")
        local ps = c:pipeline_status("jeebon-test")
        assert.eq(#ps, 3)

        assert.eq(ps[1].name, "test")
        assert.eq(ps[1].phase, "Steady")
        assert.eq(ps[1].freight, "freight-abc123")
        assert.eq(ps[1].healthy, true)

        assert.eq(ps[2].name, "dev")
        assert.eq(ps[2].phase, "Promoting")
        assert.eq(ps[2].healthy, true)

        assert.eq(ps[3].name, "prod")
        assert.eq(ps[3].phase, "Failed")
        assert.eq(ps[3].freight, "freight-old789")
        assert.eq(ps[3].healthy, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_kargo_url_trailing_slash_stripped() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/kargo.akuity.io/v1alpha1/projects/jeebon-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "kargo.akuity.io/v1alpha1",
            "kind": "Project",
            "metadata": {"name": "jeebon-test"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local kargo = require("assay.kargo")
        local c = kargo.client("{}///", "fake-token")
        local p = c:project("jeebon-test")
        assert.eq(p.metadata.name, "jeebon-test")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
