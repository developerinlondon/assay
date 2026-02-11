mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_certmanager() {
    let script = r#"
        local cm = require("assay.certmanager")
        assert.not_nil(cm)
        assert.not_nil(cm.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_certificates_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/cert-manager.io/v1/namespaces/infra/certificates",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "CertificateList",
            "items": [
                {
                    "apiVersion": "cert-manager.io/v1",
                    "kind": "Certificate",
                    "metadata": {"name": "web-tls", "namespace": "infra"},
                    "spec": {"secretName": "web-tls-secret", "issuerRef": {"name": "letsencrypt", "kind": "ClusterIssuer"}},
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True", "reason": "Ready", "message": "Certificate is up to date"}],
                        "notAfter": "2026-06-01T00:00:00Z",
                        "notBefore": "2026-03-01T00:00:00Z",
                        "renewalTime": "2026-05-01T00:00:00Z",
                        "revision": 1
                    }
                },
                {
                    "apiVersion": "cert-manager.io/v1",
                    "kind": "Certificate",
                    "metadata": {"name": "api-tls", "namespace": "infra"},
                    "spec": {"secretName": "api-tls-secret", "issuerRef": {"name": "letsencrypt", "kind": "ClusterIssuer"}},
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True", "reason": "Ready", "message": "Certificate is up to date"}],
                        "notAfter": "2026-07-01T00:00:00Z"
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cm = require("assay.certmanager")
        local c = cm.client("{}", "fake-token")
        local list = c:certificates("infra")
        assert.eq(list.kind, "CertificateList")
        assert.eq(#list.items, 2)
        assert.eq(list.items[1].metadata.name, "web-tls")
        assert.eq(list.items[2].metadata.name, "api-tls")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_certificate_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/cert-manager.io/v1/namespaces/infra/certificates/web-tls",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "Certificate",
            "metadata": {"name": "web-tls", "namespace": "infra"},
            "spec": {"secretName": "web-tls-secret", "issuerRef": {"name": "letsencrypt", "kind": "ClusterIssuer"}},
            "status": {
                "conditions": [{"type": "Ready", "status": "True", "reason": "Ready"}],
                "notAfter": "2026-06-01T00:00:00Z",
                "notBefore": "2026-03-01T00:00:00Z",
                "renewalTime": "2026-05-01T00:00:00Z",
                "revision": 1
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cm = require("assay.certmanager")
        local c = cm.client("{}", "fake-token")
        local cert = c:certificate("infra", "web-tls")
        assert.eq(cert.kind, "Certificate")
        assert.eq(cert.metadata.name, "web-tls")
        assert.eq(cert.spec.secretName, "web-tls-secret")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_certificate_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/cert-manager.io/v1/namespaces/infra/certificates/web-tls",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "Certificate",
            "metadata": {"name": "web-tls", "namespace": "infra"},
            "status": {
                "conditions": [{"type": "Ready", "status": "True", "reason": "Ready", "message": "Certificate is up to date"}],
                "notAfter": "2026-06-01T00:00:00Z",
                "notBefore": "2026-03-01T00:00:00Z",
                "renewalTime": "2026-05-01T00:00:00Z",
                "revision": 1
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cm = require("assay.certmanager")
        local c = cm.client("{}", "fake-token")
        local st = c:certificate_status("infra", "web-tls")
        assert.eq(st.ready, true)
        assert.eq(st.not_after, "2026-06-01T00:00:00Z")
        assert.eq(st.not_before, "2026-03-01T00:00:00Z")
        assert.eq(st.renewal_time, "2026-05-01T00:00:00Z")
        assert.eq(st.revision, 1)
        assert.eq(#st.conditions, 1)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_certificate_ready_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/cert-manager.io/v1/namespaces/infra/certificates/web-tls",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "Certificate",
            "metadata": {"name": "web-tls"},
            "status": {
                "conditions": [{"type": "Ready", "status": "True"}]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cm = require("assay.certmanager")
        local c = cm.client("{}", "fake-token")
        assert.eq(c:is_certificate_ready("infra", "web-tls"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_certificate_ready_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/cert-manager.io/v1/namespaces/infra/certificates/web-tls",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "Certificate",
            "metadata": {"name": "web-tls"},
            "status": {
                "conditions": [{"type": "Ready", "status": "False", "reason": "Issuing", "message": "Waiting for issuer"}]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cm = require("assay.certmanager")
        local c = cm.client("{}", "fake-token")
        assert.eq(c:is_certificate_ready("infra", "web-tls"), false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_issuers_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/cert-manager.io/v1/namespaces/infra/issuers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "IssuerList",
            "items": [
                {
                    "apiVersion": "cert-manager.io/v1",
                    "kind": "Issuer",
                    "metadata": {"name": "selfsigned", "namespace": "infra"},
                    "status": {"conditions": [{"type": "Ready", "status": "True"}]}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cm = require("assay.certmanager")
        local c = cm.client("{}", "fake-token")
        local list = c:issuers("infra")
        assert.eq(list.kind, "IssuerList")
        assert.eq(#list.items, 1)
        assert.eq(list.items[1].metadata.name, "selfsigned")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_issuer_ready() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/cert-manager.io/v1/namespaces/infra/issuers/selfsigned",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "Issuer",
            "metadata": {"name": "selfsigned", "namespace": "infra"},
            "status": {"conditions": [{"type": "Ready", "status": "True", "reason": "IsReady"}]}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cm = require("assay.certmanager")
        local c = cm.client("{}", "fake-token")
        assert.eq(c:is_issuer_ready("infra", "selfsigned"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_cluster_issuers_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/cert-manager.io/v1/clusterissuers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "ClusterIssuerList",
            "items": [
                {
                    "apiVersion": "cert-manager.io/v1",
                    "kind": "ClusterIssuer",
                    "metadata": {"name": "letsencrypt-prod"},
                    "status": {"conditions": [{"type": "Ready", "status": "True"}]}
                },
                {
                    "apiVersion": "cert-manager.io/v1",
                    "kind": "ClusterIssuer",
                    "metadata": {"name": "letsencrypt-staging"},
                    "status": {"conditions": [{"type": "Ready", "status": "True"}]}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cm = require("assay.certmanager")
        local c = cm.client("{}", "fake-token")
        local list = c:cluster_issuers()
        assert.eq(list.kind, "ClusterIssuerList")
        assert.eq(#list.items, 2)
        assert.eq(list.items[1].metadata.name, "letsencrypt-prod")
        assert.eq(list.items[2].metadata.name, "letsencrypt-staging")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_cluster_issuer_ready() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/cert-manager.io/v1/clusterissuers/letsencrypt-prod",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "ClusterIssuer",
            "metadata": {"name": "letsencrypt-prod"},
            "status": {"conditions": [{"type": "Ready", "status": "True", "reason": "ACMEAccountRegistered"}]}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cm = require("assay.certmanager")
        local c = cm.client("{}", "fake-token")
        assert.eq(c:is_cluster_issuer_ready("letsencrypt-prod"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_certificate_requests_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/cert-manager.io/v1/namespaces/infra/certificaterequests",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "CertificateRequestList",
            "items": [
                {
                    "apiVersion": "cert-manager.io/v1",
                    "kind": "CertificateRequest",
                    "metadata": {"name": "web-tls-abc12", "namespace": "infra"},
                    "status": {
                        "conditions": [
                            {"type": "Approved", "status": "True", "reason": "cert-manager.io"},
                            {"type": "Ready", "status": "True", "reason": "Issued"}
                        ]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cm = require("assay.certmanager")
        local c = cm.client("{}", "fake-token")
        local list = c:certificate_requests("infra")
        assert.eq(list.kind, "CertificateRequestList")
        assert.eq(#list.items, 1)
        assert.eq(list.items[1].metadata.name, "web-tls-abc12")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_request_approved() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/cert-manager.io/v1/namespaces/infra/certificaterequests/web-tls-abc12",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "CertificateRequest",
            "metadata": {"name": "web-tls-abc12", "namespace": "infra"},
            "status": {
                "conditions": [
                    {"type": "Approved", "status": "True", "reason": "cert-manager.io"},
                    {"type": "Ready", "status": "True", "reason": "Issued"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cm = require("assay.certmanager")
        local c = cm.client("{}", "fake-token")
        assert.eq(c:is_request_approved("infra", "web-tls-abc12"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_orders_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/acme.cert-manager.io/v1/namespaces/infra/orders",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "acme.cert-manager.io/v1",
            "kind": "OrderList",
            "items": [
                {
                    "apiVersion": "acme.cert-manager.io/v1",
                    "kind": "Order",
                    "metadata": {"name": "web-tls-order-xyz", "namespace": "infra"},
                    "status": {"state": "valid"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cm = require("assay.certmanager")
        local c = cm.client("{}", "fake-token")
        local list = c:orders("infra")
        assert.eq(list.kind, "OrderList")
        assert.eq(#list.items, 1)
        assert.eq(list.items[1].metadata.name, "web-tls-order-xyz")
        assert.eq(list.items[1].status.state, "valid")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_challenges_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/acme.cert-manager.io/v1/namespaces/infra/challenges",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "acme.cert-manager.io/v1",
            "kind": "ChallengeList",
            "items": [
                {
                    "apiVersion": "acme.cert-manager.io/v1",
                    "kind": "Challenge",
                    "metadata": {"name": "web-tls-challenge-abc", "namespace": "infra"},
                    "spec": {"type": "http-01", "dnsName": "example.com"},
                    "status": {"state": "valid", "presented": true}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cm = require("assay.certmanager")
        local c = cm.client("{}", "fake-token")
        local list = c:challenges("infra")
        assert.eq(list.kind, "ChallengeList")
        assert.eq(#list.items, 1)
        assert.eq(list.items[1].metadata.name, "web-tls-challenge-abc")
        assert.eq(list.items[1].spec.dnsName, "example.com")
        assert.eq(list.items[1].status.state, "valid")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_all_certificates_ready_all_ready() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/cert-manager.io/v1/namespaces/infra/certificates",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "CertificateList",
            "items": [
                {
                    "metadata": {"name": "web-tls"},
                    "status": {"conditions": [{"type": "Ready", "status": "True"}]}
                },
                {
                    "metadata": {"name": "api-tls"},
                    "status": {"conditions": [{"type": "Ready", "status": "True"}]}
                },
                {
                    "metadata": {"name": "grpc-tls"},
                    "status": {"conditions": [{"type": "Ready", "status": "True"}]}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cm = require("assay.certmanager")
        local c = cm.client("{}", "fake-token")
        local result = c:all_certificates_ready("infra")
        assert.eq(result.total, 3)
        assert.eq(result.ready, 3)
        assert.eq(result.not_ready, 0)
        assert.eq(#result.not_ready_names, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_all_certificates_ready_some_not_ready() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/cert-manager.io/v1/namespaces/infra/certificates",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "CertificateList",
            "items": [
                {
                    "metadata": {"name": "web-tls"},
                    "status": {"conditions": [{"type": "Ready", "status": "True"}]}
                },
                {
                    "metadata": {"name": "api-tls"},
                    "status": {"conditions": [{"type": "Ready", "status": "False", "reason": "Issuing"}]}
                },
                {
                    "metadata": {"name": "grpc-tls"},
                    "status": {"conditions": [{"type": "Ready", "status": "False", "reason": "Pending"}]}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cm = require("assay.certmanager")
        local c = cm.client("{}", "fake-token")
        local result = c:all_certificates_ready("infra")
        assert.eq(result.total, 3)
        assert.eq(result.ready, 1)
        assert.eq(result.not_ready, 2)
        assert.eq(#result.not_ready_names, 2)
        assert.eq(result.not_ready_names[1], "api-tls")
        assert.eq(result.not_ready_names[2], "grpc-tls")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_all_issuers_ready() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/cert-manager.io/v1/namespaces/infra/issuers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "IssuerList",
            "items": [
                {
                    "metadata": {"name": "selfsigned"},
                    "status": {"conditions": [{"type": "Ready", "status": "True"}]}
                },
                {
                    "metadata": {"name": "ca-issuer"},
                    "status": {"conditions": [{"type": "Ready", "status": "False", "reason": "NotReady"}]}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local cm = require("assay.certmanager")
        local c = cm.client("{}", "fake-token")
        local result = c:all_issuers_ready("infra")
        assert.eq(result.total, 2)
        assert.eq(result.ready, 1)
        assert.eq(result.not_ready, 1)
        assert.eq(result.not_ready_names[1], "ca-issuer")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
