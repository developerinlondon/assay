mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_eso() {
    let script = r#"
        local eso = require("assay.eso")
        assert.not_nil(eso)
        assert.not_nil(eso.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_external_secrets_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/external-secrets.io/v1beta1/namespaces/infra/externalsecrets",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "external-secrets.io/v1beta1",
            "kind": "ExternalSecretList",
            "items": [
                {
                    "apiVersion": "external-secrets.io/v1beta1",
                    "kind": "ExternalSecret",
                    "metadata": {"name": "db-creds", "namespace": "infra"},
                    "spec": {"secretStoreRef": {"name": "vault-backend", "kind": "SecretStore"}},
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True", "reason": "SecretSynced"}],
                        "syncedResourceVersion": "1"
                    }
                },
                {
                    "apiVersion": "external-secrets.io/v1beta1",
                    "kind": "ExternalSecret",
                    "metadata": {"name": "api-keys", "namespace": "infra"},
                    "spec": {"secretStoreRef": {"name": "vault-backend", "kind": "SecretStore"}},
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True", "reason": "SecretSynced"}],
                        "syncedResourceVersion": "2"
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local eso = require("assay.eso")
        local c = eso.client("{}", "fake-token")
        local list = c:external_secrets("infra")
        assert.eq(list.kind, "ExternalSecretList")
        assert.eq(#list.items, 2)
        assert.eq(list.items[1].metadata.name, "db-creds")
        assert.eq(list.items[2].metadata.name, "api-keys")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_external_secret_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/external-secrets.io/v1beta1/namespaces/infra/externalsecrets/db-creds",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "external-secrets.io/v1beta1",
            "kind": "ExternalSecret",
            "metadata": {"name": "db-creds", "namespace": "infra"},
            "spec": {
                "refreshInterval": "1h",
                "secretStoreRef": {"name": "vault-backend", "kind": "SecretStore"},
                "target": {"name": "db-creds"},
                "data": [{"secretKey": "password", "remoteRef": {"key": "secret/db", "property": "password"}}]
            },
            "status": {
                "conditions": [{"type": "Ready", "status": "True", "reason": "SecretSynced"}],
                "syncedResourceVersion": "abc123"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local eso = require("assay.eso")
        local c = eso.client("{}", "fake-token")
        local es = c:external_secret("infra", "db-creds")
        assert.eq(es.kind, "ExternalSecret")
        assert.eq(es.metadata.name, "db-creds")
        assert.eq(es.spec.secretStoreRef.name, "vault-backend")
        assert.eq(es.status.syncedResourceVersion, "abc123")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_external_secret_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/external-secrets.io/v1beta1/namespaces/infra/externalsecrets/db-creds",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "external-secrets.io/v1beta1",
            "kind": "ExternalSecret",
            "metadata": {"name": "db-creds", "namespace": "infra"},
            "status": {
                "conditions": [
                    {"type": "Ready", "status": "True", "reason": "SecretSynced", "message": "Secret synced"}
                ],
                "syncedResourceVersion": "v42"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local eso = require("assay.eso")
        local c = eso.client("{}", "fake-token")
        local st = c:external_secret_status("infra", "db-creds")
        assert.eq(st.ready, true)
        assert.eq(st.status, "SecretSynced")
        assert.eq(st.sync_hash, "v42")
        assert.eq(#st.conditions, 1)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_secret_synced_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/external-secrets.io/v1beta1/namespaces/infra/externalsecrets/db-creds",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "external-secrets.io/v1beta1",
            "kind": "ExternalSecret",
            "metadata": {"name": "db-creds", "namespace": "infra"},
            "status": {
                "conditions": [{"type": "Ready", "status": "True", "reason": "SecretSynced"}]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local eso = require("assay.eso")
        local c = eso.client("{}", "fake-token")
        assert.eq(c:is_secret_synced("infra", "db-creds"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_secret_synced_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/external-secrets.io/v1beta1/namespaces/infra/externalsecrets/db-creds",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "external-secrets.io/v1beta1",
            "kind": "ExternalSecret",
            "metadata": {"name": "db-creds", "namespace": "infra"},
            "status": {
                "conditions": [{"type": "Ready", "status": "False", "reason": "SecretSyncError", "message": "vault unreachable"}]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local eso = require("assay.eso")
        local c = eso.client("{}", "fake-token")
        assert.eq(c:is_secret_synced("infra", "db-creds"), false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_secret_stores_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/external-secrets.io/v1beta1/namespaces/infra/secretstores",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "external-secrets.io/v1beta1",
            "kind": "SecretStoreList",
            "items": [
                {
                    "apiVersion": "external-secrets.io/v1beta1",
                    "kind": "SecretStore",
                    "metadata": {"name": "vault-backend", "namespace": "infra"},
                    "spec": {"provider": {"vault": {"server": "https://vault.infra:8200"}}},
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True", "reason": "Valid"}]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local eso = require("assay.eso")
        local c = eso.client("{}", "fake-token")
        local list = c:secret_stores("infra")
        assert.eq(list.kind, "SecretStoreList")
        assert.eq(#list.items, 1)
        assert.eq(list.items[1].metadata.name, "vault-backend")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_secret_store_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/external-secrets.io/v1beta1/namespaces/infra/secretstores/vault-backend",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "external-secrets.io/v1beta1",
            "kind": "SecretStore",
            "metadata": {"name": "vault-backend", "namespace": "infra"},
            "status": {
                "conditions": [
                    {"type": "Ready", "status": "True", "reason": "Valid", "message": "store validated"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local eso = require("assay.eso")
        local c = eso.client("{}", "fake-token")
        local st = c:secret_store_status("infra", "vault-backend")
        assert.eq(st.ready, true)
        assert.eq(#st.conditions, 1)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_store_ready() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/external-secrets.io/v1beta1/namespaces/infra/secretstores/vault-backend",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "external-secrets.io/v1beta1",
            "kind": "SecretStore",
            "metadata": {"name": "vault-backend", "namespace": "infra"},
            "status": {
                "conditions": [{"type": "Ready", "status": "True", "reason": "Valid"}]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local eso = require("assay.eso")
        local c = eso.client("{}", "fake-token")
        assert.eq(c:is_store_ready("infra", "vault-backend"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_cluster_secret_stores_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/external-secrets.io/v1beta1/clustersecretstores",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "external-secrets.io/v1beta1",
            "kind": "ClusterSecretStoreList",
            "items": [
                {
                    "apiVersion": "external-secrets.io/v1beta1",
                    "kind": "ClusterSecretStore",
                    "metadata": {"name": "global-vault"},
                    "spec": {"provider": {"vault": {"server": "https://vault.infra:8200"}}},
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True", "reason": "Valid"}]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local eso = require("assay.eso")
        local c = eso.client("{}", "fake-token")
        local list = c:cluster_secret_stores()
        assert.eq(list.kind, "ClusterSecretStoreList")
        assert.eq(#list.items, 1)
        assert.eq(list.items[1].metadata.name, "global-vault")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_is_cluster_store_ready() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/external-secrets.io/v1beta1/clustersecretstores/global-vault",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "external-secrets.io/v1beta1",
            "kind": "ClusterSecretStore",
            "metadata": {"name": "global-vault"},
            "status": {
                "conditions": [{"type": "Ready", "status": "True", "reason": "Valid"}]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local eso = require("assay.eso")
        local c = eso.client("{}", "fake-token")
        assert.eq(c:is_cluster_store_ready("global-vault"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_all_secrets_synced_all_synced() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/external-secrets.io/v1beta1/namespaces/infra/externalsecrets",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "external-secrets.io/v1beta1",
            "kind": "ExternalSecretList",
            "items": [
                {
                    "metadata": {"name": "db-creds", "namespace": "infra"},
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True", "reason": "SecretSynced"}]
                    }
                },
                {
                    "metadata": {"name": "api-keys", "namespace": "infra"},
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True", "reason": "SecretSynced"}]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local eso = require("assay.eso")
        local c = eso.client("{}", "fake-token")
        local result = c:all_secrets_synced("infra")
        assert.eq(result.synced, 2)
        assert.eq(result.failed, 0)
        assert.eq(result.total, 2)
        assert.eq(#result.failed_names, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_all_secrets_synced_some_failed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/external-secrets.io/v1beta1/namespaces/infra/externalsecrets",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "external-secrets.io/v1beta1",
            "kind": "ExternalSecretList",
            "items": [
                {
                    "metadata": {"name": "db-creds", "namespace": "infra"},
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True", "reason": "SecretSynced"}]
                    }
                },
                {
                    "metadata": {"name": "broken-secret", "namespace": "infra"},
                    "status": {
                        "conditions": [{"type": "Ready", "status": "False", "reason": "SecretSyncError"}]
                    }
                },
                {
                    "metadata": {"name": "missing-ref", "namespace": "infra"},
                    "status": {
                        "conditions": [{"type": "Ready", "status": "False", "reason": "SecretSyncError"}]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local eso = require("assay.eso")
        local c = eso.client("{}", "fake-token")
        local result = c:all_secrets_synced("infra")
        assert.eq(result.synced, 1)
        assert.eq(result.failed, 2)
        assert.eq(result.total, 3)
        assert.eq(#result.failed_names, 2)
        assert.eq(result.failed_names[1], "broken-secret")
        assert.eq(result.failed_names[2], "missing-ref")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_all_stores_ready() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/external-secrets.io/v1beta1/namespaces/infra/secretstores",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "external-secrets.io/v1beta1",
            "kind": "SecretStoreList",
            "items": [
                {
                    "metadata": {"name": "vault-backend", "namespace": "infra"},
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True", "reason": "Valid"}]
                    }
                },
                {
                    "metadata": {"name": "broken-store", "namespace": "infra"},
                    "status": {
                        "conditions": [{"type": "Ready", "status": "False", "reason": "ConfigError"}]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local eso = require("assay.eso")
        local c = eso.client("{}", "fake-token")
        local result = c:all_stores_ready("infra")
        assert.eq(result.ready, 1)
        assert.eq(result.not_ready, 1)
        assert.eq(result.total, 2)
        assert.eq(#result.not_ready_names, 1)
        assert.eq(result.not_ready_names[1], "broken-store")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_cluster_external_secrets_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/external-secrets.io/v1beta1/clusterexternalsecrets",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "external-secrets.io/v1beta1",
            "kind": "ClusterExternalSecretList",
            "items": [
                {
                    "apiVersion": "external-secrets.io/v1beta1",
                    "kind": "ClusterExternalSecret",
                    "metadata": {"name": "global-db-creds"},
                    "spec": {
                        "externalSecretSpec": {
                            "secretStoreRef": {"name": "global-vault", "kind": "ClusterSecretStore"}
                        },
                        "namespaceSelector": {"matchLabels": {"env": "production"}}
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local eso = require("assay.eso")
        local c = eso.client("{}", "fake-token")
        local list = c:cluster_external_secrets()
        assert.eq(list.kind, "ClusterExternalSecretList")
        assert.eq(#list.items, 1)
        assert.eq(list.items[1].metadata.name, "global-db-creds")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_cluster_external_secret_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/external-secrets.io/v1beta1/clusterexternalsecrets/global-db-creds",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "external-secrets.io/v1beta1",
            "kind": "ClusterExternalSecret",
            "metadata": {"name": "global-db-creds"},
            "spec": {
                "externalSecretSpec": {
                    "secretStoreRef": {"name": "global-vault", "kind": "ClusterSecretStore"}
                }
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local eso = require("assay.eso")
        local c = eso.client("{}", "fake-token")
        local ces = c:cluster_external_secret("global-db-creds")
        assert.eq(ces.kind, "ClusterExternalSecret")
        assert.eq(ces.metadata.name, "global-db-creds")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_client_strips_trailing_slashes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/external-secrets.io/v1beta1/clustersecretstores/test",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "external-secrets.io/v1beta1",
            "kind": "ClusterSecretStore",
            "metadata": {"name": "test"},
            "status": {
                "conditions": [{"type": "Ready", "status": "True", "reason": "Valid"}]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local eso = require("assay.eso")
        local c = eso.client("{}///", "fake-token")
        assert.eq(c:is_cluster_store_ready("test"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
