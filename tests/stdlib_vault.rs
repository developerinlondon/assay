mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_vault() {
    let script = r#"
        local vault = require("assay.vault")
        assert.not_nil(vault)
        assert.not_nil(vault.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_vault_read() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/mykey"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {"data": {"username": "admin", "password": "secret123"}}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local data = c:read("secret/data/mykey")
        assert.eq(data.data.username, "admin")
        assert.eq(data.data.password, "secret123")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_read_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local data = c:read("secret/data/missing")
        assert.eq(data, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_write() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/secret/data/newkey"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        c:write("secret/data/newkey", {{ data = {{ key = "value" }} }})
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_delete() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/v1/secret/data/oldkey"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        c:delete("secret/data/oldkey")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/metadata"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {"keys": ["key1", "key2", "key3"]}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local keys = c:list("secret/metadata")
        assert.eq(#keys, 3)
        assert.eq(keys[1], "key1")
        assert.eq(keys[3], "key3")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_list_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/metadata"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local keys = c:list("secret/metadata")
        assert.eq(#keys, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_kv_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/mykey"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {"data": {"foo": "bar"}}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local data = c:kv_get("secret", "mykey")
        assert.eq(data.data.foo, "bar")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_kv_put() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/secret/data/mykey"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        c:kv_put("secret", "mykey", {{ username = "admin", password = "s3cret" }})
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_kv_delete() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/v1/secret/data/mykey"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        c:kv_delete("secret", "mykey")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_kv_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/metadata/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {"keys": ["db-creds", "api-keys", "tls/"]}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local keys = c:kv_list("secret")
        assert.eq(#keys, 3)
        assert.eq(keys[1], "db-creds")
        assert.eq(keys[3], "tls/")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_kv_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/metadata/mykey"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "created_time": "2026-01-15T10:30:00Z",
                "current_version": 3,
                "max_versions": 10,
                "oldest_version": 1,
                "versions": {
                    "1": {"created_time": "2026-01-10T08:00:00Z", "deletion_time": ""},
                    "2": {"created_time": "2026-01-12T09:00:00Z", "deletion_time": ""},
                    "3": {"created_time": "2026-01-15T10:30:00Z", "deletion_time": ""}
                }
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local meta = c:kv_metadata("secret", "mykey")
        assert.eq(meta.data.current_version, 3)
        assert.eq(meta.data.max_versions, 10)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_health() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sys/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "initialized": true,
            "sealed": false,
            "standby": false,
            "performance_standby": false,
            "replication_performance_mode": "disabled",
            "replication_dr_mode": "disabled",
            "server_time_utc": 1700000000,
            "version": "1.15.0",
            "cluster_name": "vault-cluster-abc123",
            "cluster_id": "550e8400-e29b-41d4-a716-446655440000"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local h = c:health()
        assert.eq(h.initialized, true)
        assert.eq(h.sealed, false)
        assert.eq(h.version, "1.15.0")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_seal_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sys/seal-status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "type": "shamir",
            "initialized": true,
            "sealed": false,
            "t": 3,
            "n": 5,
            "progress": 0,
            "nonce": "",
            "version": "1.15.0",
            "build_date": "2026-01-01T00:00:00Z",
            "migration": false,
            "cluster_name": "vault-cluster-abc123",
            "cluster_id": "550e8400-e29b-41d4-a716-446655440000",
            "recovery_seal": false,
            "storage_type": "raft"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local s = c:seal_status()
        assert.eq(s.sealed, false)
        assert.eq(s.initialized, true)
        assert.eq(s.t, 3)
        assert.eq(s.n, 5)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_is_sealed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sys/seal-status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "sealed": true,
            "initialized": true,
            "t": 3,
            "n": 5,
            "progress": 1
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        assert.eq(c:is_sealed(), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_is_initialized() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sys/seal-status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "sealed": false,
            "initialized": true,
            "t": 3,
            "n": 5,
            "progress": 0
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        assert.eq(c:is_initialized(), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_policy_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sys/policies/acl/my-policy"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "name": "my-policy",
                "rules": "path \"secret/data/*\" {\n  capabilities = [\"read\"]\n}"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local pol = c:policy_get("my-policy")
        assert.eq(pol.name, "my-policy")
        assert.contains(pol.rules, "secret/data")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_policy_put() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/v1/sys/policies/acl/my-policy"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        c:policy_put("my-policy", 'path "secret/data/*" {{ capabilities = ["read"] }}')
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_policy_delete() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/v1/sys/policies/acl/my-policy"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        c:policy_delete("my-policy")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_policy_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sys/policies/acl"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {"keys": ["default", "root", "my-policy"]}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local policies = c:policy_list()
        assert.eq(#policies, 3)
        assert.eq(policies[1], "default")
        assert.eq(policies[3], "my-policy")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_auth_enable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/sys/auth/kubernetes"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        c:auth_enable("kubernetes", "kubernetes", {{ description = "K8s auth" }})
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_auth_disable() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/v1/sys/auth/kubernetes"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        c:auth_disable("kubernetes")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_auth_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sys/auth"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "token/": {
                    "type": "token",
                    "description": "token based credentials"
                },
                "kubernetes/": {
                    "type": "kubernetes",
                    "description": "K8s auth"
                }
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local auths = c:auth_list()
        assert.not_nil(auths["token/"])
        assert.eq(auths["token/"].type, "token")
        assert.eq(auths["kubernetes/"].type, "kubernetes")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_auth_config() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/auth/kubernetes/config"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        c:auth_config("kubernetes", {{
            kubernetes_host = "https://kubernetes.default.svc",
        }})
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_auth_create_role() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/auth/kubernetes/role/my-app"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        c:auth_create_role("kubernetes", "my-app", {{
            bound_service_account_names = {{ "my-app" }},
            bound_service_account_namespaces = {{ "default" }},
            policies = {{ "my-policy" }},
            ttl = "1h",
        }})
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_auth_read_role() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/auth/kubernetes/role/my-app"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "bound_service_account_names": ["my-app"],
                "bound_service_account_namespaces": ["default"],
                "policies": ["my-policy"],
                "ttl": 3600,
                "max_ttl": 86400
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local role = c:auth_read_role("kubernetes", "my-app")
        assert.eq(role.policies[1], "my-policy")
        assert.eq(role.ttl, 3600)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_auth_list_roles() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/auth/kubernetes/role"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {"keys": ["my-app", "worker", "admin"]}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local roles = c:auth_list_roles("kubernetes")
        assert.eq(#roles, 3)
        assert.eq(roles[1], "my-app")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_engine_enable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/sys/mounts/transit"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        c:engine_enable("transit", "transit", {{ description = "Encryption as a service" }})
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_engine_disable() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/v1/sys/mounts/transit"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        c:engine_disable("transit")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_engine_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sys/mounts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "secret/": {
                    "type": "kv",
                    "options": {"version": "2"}
                },
                "transit/": {
                    "type": "transit",
                    "description": "Encryption as a service"
                },
                "pki/": {
                    "type": "pki",
                    "description": "PKI engine"
                }
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local engines = c:engine_list()
        assert.not_nil(engines["secret/"])
        assert.eq(engines["secret/"].type, "kv")
        assert.eq(engines["transit/"].type, "transit")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_engine_tune() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/sys/mounts/secret/tune"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        c:engine_tune("secret", {{ max_lease_ttl = "87600h", default_lease_ttl = "1h" }})
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_token_create() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/auth/token/create"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "auth": {
                "client_token": "hvs.CAESI_new_child_token",
                "accessor": "accessor-abc123",
                "policies": ["default", "my-policy"],
                "token_policies": ["default", "my-policy"],
                "metadata": {},
                "lease_duration": 3600,
                "renewable": true
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local auth = c:token_create({{ policies = {{ "my-policy" }}, ttl = "1h" }})
        assert.eq(auth.client_token, "hvs.CAESI_new_child_token")
        assert.eq(auth.renewable, true)
        assert.eq(auth.lease_duration, 3600)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_token_lookup() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/auth/token/lookup"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "accessor": "accessor-abc123",
                "creation_time": 1700000000,
                "creation_ttl": 3600,
                "display_name": "token",
                "expire_time": "2026-02-10T12:00:00Z",
                "id": "hvs.CAESI_some_token",
                "policies": ["default", "my-policy"],
                "renewable": true,
                "ttl": 1800
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local info = c:token_lookup("hvs.CAESI_some_token")
        assert.eq(info.id, "hvs.CAESI_some_token")
        assert.eq(info.policies[2], "my-policy")
        assert.eq(info.renewable, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_token_lookup_self() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/auth/token/lookup-self"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "accessor": "accessor-self-123",
                "creation_time": 1700000000,
                "id": "hvs.CAESI_self_token",
                "policies": ["default", "admin"],
                "ttl": 7200
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local info = c:token_lookup_self()
        assert.eq(info.id, "hvs.CAESI_self_token")
        assert.eq(info.policies[2], "admin")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_token_revoke() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/auth/token/revoke"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        c:token_revoke("hvs.CAESI_revoke_me")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_token_revoke_self() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/auth/token/revoke-self"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        c:token_revoke_self()
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_transit_encrypt() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/transit/encrypt/my-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "ciphertext": "vault:v1:ABCDEF1234567890encrypted"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local ct = c:transit_encrypt("my-key", "hello world")
        assert.eq(ct, "vault:v1:ABCDEF1234567890encrypted")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_transit_decrypt() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/transit/decrypt/my-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "plaintext": "aGVsbG8gd29ybGQ="
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local pt = c:transit_decrypt("my-key", "vault:v1:ABCDEF1234567890encrypted")
        assert.eq(pt, "hello world")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_transit_create_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/transit/keys/new-key"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        c:transit_create_key("new-key", {{ type = "aes256-gcm96" }})
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_transit_list_keys() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/transit/keys"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {"keys": ["my-key", "backup-key", "signing-key"]}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local keys = c:transit_list_keys()
        assert.eq(#keys, 3)
        assert.eq(keys[1], "my-key")
        assert.eq(keys[3], "signing-key")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_pki_issue() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/pki/issue/web-certs"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {
                    "certificate": "-----BEGIN CERTIFICATE-----\nMIIB...\n-----END CERTIFICATE-----",
                    "issuing_ca": "-----BEGIN CERTIFICATE-----\nMIIC...\n-----END CERTIFICATE-----",
                    "private_key": "-----BEGIN RSA PRIVATE KEY-----\nMIIE...\n-----END RSA PRIVATE KEY-----",
                    "private_key_type": "rsa",
                    "serial_number": "39:dd:2e:90:b7:23:1f:8d:d3:7d:31:c5"
                }
            })),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local cert = c:pki_issue("pki", "web-certs", {{ common_name = "example.com", ttl = "720h" }})
        assert.contains(cert.certificate, "BEGIN CERTIFICATE")
        assert.contains(cert.private_key, "BEGIN RSA PRIVATE KEY")
        assert.eq(cert.private_key_type, "rsa")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_pki_ca_cert() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/pki/ca/pem"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                "-----BEGIN CERTIFICATE-----\nMIICpDCCAYwCCQC7fC0bJNvDPDANBgkqhkiG9w0BAQsFADAU\n-----END CERTIFICATE-----\n",
            ),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        local pem = c:pki_ca_cert("pki")
        assert.contains(pem, "BEGIN CERTIFICATE")
        assert.contains(pem, "END CERTIFICATE")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_vault_pki_create_role() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/pki/roles/web-certs"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.vault")
        local c = vault.client("{}", "test-token")
        c:pki_create_role("pki", "web-certs", {{
            allowed_domains = {{ "example.com" }},
            allow_subdomains = true,
            max_ttl = "720h",
        }})
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
