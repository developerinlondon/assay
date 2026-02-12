mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_crossplane() {
    let script = r#"
        local cp = require("assay.crossplane")
        assert.not_nil(cp)
        assert.not_nil(cp.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_providers_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/pkg.crossplane.io/v1/providers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "ProviderList",
            "items": [
                {
                    "metadata": {"name": "provider-aws-s3"},
                    "status": {
                        "conditions": [
                            {"type": "Installed", "status": "True"},
                            {"type": "Healthy", "status": "True"}
                        ]
                    }
                },
                {
                    "metadata": {"name": "provider-aws-ec2"},
                    "status": {
                        "conditions": [
                            {"type": "Installed", "status": "True"},
                            {"type": "Healthy", "status": "True"}
                        ]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        local list = c:providers()
        assert.eq(list.kind, "ProviderList")
        assert.eq(#list.items, 2)
        assert.eq(list.items[1].metadata.name, "provider-aws-s3")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_provider_single() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/pkg.crossplane.io/v1/providers/provider-aws-s3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "Provider",
            "metadata": {"name": "provider-aws-s3"},
            "spec": {"package": "xpkg.upbound.io/upbound/provider-aws-s3:v1.2.0"},
            "status": {
                "currentRevision": "provider-aws-s3-abc123",
                "conditions": [
                    {"type": "Installed", "status": "True"},
                    {"type": "Healthy", "status": "True"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        local p = c:provider("provider-aws-s3")
        assert.eq(p.metadata.name, "provider-aws-s3")
        assert.eq(p.status.currentRevision, "provider-aws-s3-abc123")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_provider_healthy_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/pkg.crossplane.io/v1/providers/provider-aws-s3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "provider-aws-s3"},
            "status": {
                "conditions": [
                    {"type": "Installed", "status": "True"},
                    {"type": "Healthy", "status": "True"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        assert.eq(c:is_provider_healthy("provider-aws-s3"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_provider_healthy_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/pkg.crossplane.io/v1/providers/provider-aws-s3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "provider-aws-s3"},
            "status": {
                "conditions": [
                    {"type": "Installed", "status": "True"},
                    {"type": "Healthy", "status": "False", "reason": "UnhealthyPackageRevision"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        assert.eq(c:is_provider_healthy("provider-aws-s3"), false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_provider_installed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/pkg.crossplane.io/v1/providers/provider-aws-s3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "provider-aws-s3"},
            "status": {
                "conditions": [
                    {"type": "Installed", "status": "True"},
                    {"type": "Healthy", "status": "True"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        assert.eq(c:is_provider_installed("provider-aws-s3"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_provider_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/pkg.crossplane.io/v1/providers/provider-aws-s3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "provider-aws-s3"},
            "status": {
                "currentRevision": "provider-aws-s3-rev1",
                "conditions": [
                    {"type": "Installed", "status": "True"},
                    {"type": "Healthy", "status": "True"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        local s = c:provider_status("provider-aws-s3")
        assert.eq(s.installed, true)
        assert.eq(s.healthy, true)
        assert.eq(s.current_revision, "provider-aws-s3-rev1")
        assert.eq(#s.conditions, 2)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_configurations_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/pkg.crossplane.io/v1/configurations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "ConfigurationList",
            "items": [
                {
                    "metadata": {"name": "platform-ref-aws"},
                    "status": {
                        "conditions": [
                            {"type": "Installed", "status": "True"},
                            {"type": "Healthy", "status": "True"}
                        ]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        local list = c:configurations()
        assert.eq(list.kind, "ConfigurationList")
        assert.eq(#list.items, 1)
        assert.eq(list.items[1].metadata.name, "platform-ref-aws")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_configuration_healthy() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/pkg.crossplane.io/v1/configurations/platform-ref-aws",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "platform-ref-aws"},
            "status": {
                "conditions": [
                    {"type": "Installed", "status": "True"},
                    {"type": "Healthy", "status": "True"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        assert.eq(c:is_configuration_healthy("platform-ref-aws"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_functions_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/pkg.crossplane.io/v1beta1/functions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "FunctionList",
            "items": [
                {
                    "metadata": {"name": "function-patch-and-transform"},
                    "status": {
                        "conditions": [
                            {"type": "Installed", "status": "True"},
                            {"type": "Healthy", "status": "True"}
                        ]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        local list = c:functions()
        assert.eq(list.kind, "FunctionList")
        assert.eq(#list.items, 1)
        assert.eq(list.items[1].metadata.name, "function-patch-and-transform")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_function_healthy() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/pkg.crossplane.io/v1beta1/functions/function-patch-and-transform",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "function-patch-and-transform"},
            "status": {
                "conditions": [
                    {"type": "Installed", "status": "True"},
                    {"type": "Healthy", "status": "True"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        assert.eq(c:is_function_healthy("function-patch-and-transform"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_xrds_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/apiextensions.crossplane.io/v1/compositeresourcedefinitions",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "CompositeResourceDefinitionList",
            "items": [
                {
                    "metadata": {"name": "xpostgresqlinstances.database.example.org"},
                    "status": {
                        "conditions": [
                            {"type": "Established", "status": "True"},
                            {"type": "Offered", "status": "True"}
                        ]
                    }
                },
                {
                    "metadata": {"name": "xnetworks.network.example.org"},
                    "status": {
                        "conditions": [
                            {"type": "Established", "status": "True"}
                        ]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        local list = c:xrds()
        assert.eq(list.kind, "CompositeResourceDefinitionList")
        assert.eq(#list.items, 2)
        assert.eq(list.items[1].metadata.name, "xpostgresqlinstances.database.example.org")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_xrd_established() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/apiextensions.crossplane.io/v1/compositeresourcedefinitions/xpostgresqlinstances.database.example.org",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "xpostgresqlinstances.database.example.org"},
            "status": {
                "conditions": [
                    {"type": "Established", "status": "True"},
                    {"type": "Offered", "status": "True"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        assert.eq(c:is_xrd_established("xpostgresqlinstances.database.example.org"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_compositions_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/apiextensions.crossplane.io/v1/compositions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "CompositionList",
            "items": [
                {
                    "metadata": {"name": "postgresqlinstance-composition"},
                    "spec": {
                        "compositeTypeRef": {
                            "apiVersion": "database.example.org/v1alpha1",
                            "kind": "XPostgreSQLInstance"
                        }
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        local list = c:compositions()
        assert.eq(list.kind, "CompositionList")
        assert.eq(#list.items, 1)
        assert.eq(list.items[1].metadata.name, "postgresqlinstance-composition")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_managed_resource() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/s3.aws.upbound.io/v1beta1/buckets/my-bucket"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "Bucket",
            "metadata": {"name": "my-bucket"},
            "status": {
                "conditions": [
                    {"type": "Ready", "status": "True"},
                    {"type": "Synced", "status": "True"}
                ],
                "atProvider": {
                    "arn": "arn:aws:s3:::my-bucket",
                    "region": "us-east-1"
                }
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        local r = c:managed_resource("s3.aws.upbound.io", "v1beta1", "buckets", "my-bucket")
        assert.eq(r.kind, "Bucket")
        assert.eq(r.metadata.name, "my-bucket")
        assert.eq(r.status.atProvider.region, "us-east-1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_managed_ready_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/s3.aws.upbound.io/v1beta1/buckets/my-bucket"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "my-bucket"},
            "status": {
                "conditions": [
                    {"type": "Ready", "status": "True"},
                    {"type": "Synced", "status": "True"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        assert.eq(c:is_managed_ready("s3.aws.upbound.io", "v1beta1", "buckets", "my-bucket"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_managed_ready_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/s3.aws.upbound.io/v1beta1/buckets/my-bucket"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "my-bucket"},
            "status": {
                "conditions": [
                    {"type": "Ready", "status": "False", "reason": "ReconcileError"},
                    {"type": "Synced", "status": "False"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        assert.eq(c:is_managed_ready("s3.aws.upbound.io", "v1beta1", "buckets", "my-bucket"), false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_all_providers_healthy() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/pkg.crossplane.io/v1/providers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "ProviderList",
            "items": [
                {
                    "metadata": {"name": "provider-aws-s3"},
                    "status": {
                        "conditions": [
                            {"type": "Healthy", "status": "True"}
                        ]
                    }
                },
                {
                    "metadata": {"name": "provider-aws-ec2"},
                    "status": {
                        "conditions": [
                            {"type": "Healthy", "status": "True"}
                        ]
                    }
                },
                {
                    "metadata": {"name": "provider-aws-rds"},
                    "status": {
                        "conditions": [
                            {"type": "Healthy", "status": "False"}
                        ]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        local result = c:all_providers_healthy()
        assert.eq(result.healthy, 2)
        assert.eq(result.unhealthy, 1)
        assert.eq(result.total, 3)
        assert.eq(result.unhealthy_names[1], "provider-aws-rds")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_all_xrds_established() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/apiextensions.crossplane.io/v1/compositeresourcedefinitions",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "CompositeResourceDefinitionList",
            "items": [
                {
                    "metadata": {"name": "xpostgresqlinstances.database.example.org"},
                    "status": {
                        "conditions": [
                            {"type": "Established", "status": "True"}
                        ]
                    }
                },
                {
                    "metadata": {"name": "xnetworks.network.example.org"},
                    "status": {
                        "conditions": [
                            {"type": "Established", "status": "False"}
                        ]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        local result = c:all_xrds_established()
        assert.eq(result.established, 1)
        assert.eq(result.not_established, 1)
        assert.eq(result.total, 2)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_managed_resources_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/s3.aws.upbound.io/v1beta1/buckets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "BucketList",
            "items": [
                {"metadata": {"name": "bucket-a"}},
                {"metadata": {"name": "bucket-b"}}
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        local list = c:managed_resources("s3.aws.upbound.io", "v1beta1", "buckets")
        assert.eq(list.kind, "BucketList")
        assert.eq(#list.items, 2)
        assert.eq(list.items[1].metadata.name, "bucket-a")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_trailing_slash_stripped() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/pkg.crossplane.io/v1/providers/test-provider"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "test-provider"},
            "status": {
                "conditions": [{"type": "Healthy", "status": "True"}]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}///", "fake-token")
        assert.eq(c:is_provider_healthy("test-provider"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_provider_revisions_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/pkg.crossplane.io/v1/providerrevisions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "ProviderRevisionList",
            "items": [
                {
                    "metadata": {"name": "provider-aws-s3-abc123"},
                    "spec": {"desiredState": "Active"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        local list = c:provider_revisions()
        assert.eq(list.kind, "ProviderRevisionList")
        assert.eq(#list.items, 1)
        assert.eq(list.items[1].spec.desiredState, "Active")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_xrd_not_established() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/apiextensions.crossplane.io/v1/compositeresourcedefinitions/xbroken.example.org",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": {"name": "xbroken.example.org"},
            "status": {
                "conditions": [
                    {"type": "Established", "status": "False", "reason": "RenderingCompositeResourceDefinition"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        assert.eq(c:is_xrd_established("xbroken.example.org"), false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_composition_single() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/apiextensions.crossplane.io/v1/compositions/my-composition",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "kind": "Composition",
            "metadata": {"name": "my-composition"},
            "spec": {
                "compositeTypeRef": {
                    "apiVersion": "database.example.org/v1alpha1",
                    "kind": "XPostgreSQLInstance"
                },
                "mode": "Pipeline"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cp = require("assay.crossplane")
        local c = cp.client("{}", "fake-token")
        local comp = c:composition("my-composition")
        assert.eq(comp.kind, "Composition")
        assert.eq(comp.spec.mode, "Pipeline")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
