mod common;

use common::run_lua;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Kubeconfig-context support: k8s calls can target any cluster in a kubeconfig
// via opts.context / k8s.use_context(), with static-token users honored as-is
// and the aws `eks get-token` exec plugin minted in-process (no subprocess —
// works in readonly mode).

fn write_kubeconfig(dir: &std::path::Path, server_uri: &str) -> std::path::PathBuf {
    let kubeconfig = format!(
        r#"apiVersion: v1
kind: Config
current-context: static-ctx
clusters:
  - name: static-cluster
    cluster:
      server: {server_uri}
  - name: eks-cluster
    cluster:
      server: {server_uri}
contexts:
  - name: static-ctx
    context:
      cluster: static-cluster
      user: static-user
  - name: eks-ctx
    context:
      cluster: eks-cluster
      user: eks-user
  - name: broken-ctx
    context:
      cluster: static-cluster
      user: exotic-user
users:
  - name: static-user
    user:
      token: static-bearer-token
  - name: eks-user
    user:
      exec:
        apiVersion: client.authentication.k8s.io/v1beta1
        command: aws
        args:
          - --region
          - us-east-1
          - eks
          - get-token
          - --cluster-name
          - my-eks-cluster
        env:
          - name: AWS_ACCESS_KEY_ID
            value: AKIDEXEC
          - name: AWS_SECRET_ACCESS_KEY
            value: execsecret
  - name: exotic-user
    user:
      exec:
        apiVersion: client.authentication.k8s.io/v1beta1
        command: gke-gcloud-auth-plugin
"#
    );
    let path = dir.join("kubeconfig");
    std::fs::write(&path, kubeconfig).unwrap();
    path
}

#[tokio::test]
async fn test_context_with_static_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces"))
        .and(header("Authorization", "Bearer static-bearer-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "items": [] })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let kc = write_kubeconfig(dir.path(), &server.uri());

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local out = k8s.get("/api/v1/namespaces", {{ context = "static-ctx", kubeconfig = "{kc}" }})
        assert.not_nil(out.items)
        "#,
        kc = kc.display()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_context_with_aws_exec_plugin_mints_token_in_process() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "items": [] })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let kc = write_kubeconfig(dir.path(), &server.uri());

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local out = k8s.get("/api/v1/namespaces", {{ context = "eks-ctx", kubeconfig = "{kc}" }})
        assert.not_nil(out.items)
        "#,
        kc = kc.display()
    );
    run_lua(&script).await.unwrap();

    // The minted bearer token must be an in-process k8s-aws-v1 EKS token.
    let requests = server.received_requests().await.unwrap();
    assert!(!requests.is_empty());
    let auth = requests[0]
        .headers
        .get("authorization")
        .expect("authorization header")
        .to_str()
        .unwrap();
    assert!(
        auth.starts_with("Bearer k8s-aws-v1."),
        "expected an in-process EKS token, got: {auth}"
    );
}

#[tokio::test]
async fn test_use_context_sets_default() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/pods"))
        .and(header("Authorization", "Bearer static-bearer-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "items": [] })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let kc = write_kubeconfig(dir.path(), &server.uri());

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        k8s.use_context("static-ctx")
        local out = k8s.get("/api/v1/pods", {{ kubeconfig = "{kc}" }})
        assert.not_nil(out.items)
        "#,
        kc = kc.display()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_contexts_listing() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let kc = write_kubeconfig(dir.path(), &server.uri());

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local names, current = k8s.contexts({{ kubeconfig = "{kc}" }})
        assert.eq(#names, 3)
        assert.eq(names[1], "static-ctx")
        assert.eq(names[2], "eks-ctx")
        assert.eq(current, "static-ctx")
        "#,
        kc = kc.display()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unknown_context_errors() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let kc = write_kubeconfig(dir.path(), &server.uri());

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local ok, err = pcall(k8s.get, "/api/v1/pods", {{ context = "ghost", kubeconfig = "{kc}" }})
        assert.eq(ok, false)
        assert.contains(tostring(err), "context 'ghost' not found")
        "#,
        kc = kc.display()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unsupported_exec_plugin_errors() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let kc = write_kubeconfig(dir.path(), &server.uri());

    let script = format!(
        r#"
        local k8s = require("assay.k8s")
        local ok, err = pcall(k8s.get, "/api/v1/pods", {{ context = "broken-ctx", kubeconfig = "{kc}" }})
        assert.eq(ok, false)
        assert.contains(tostring(err), "unsupported exec credential plugin")
        "#,
        kc = kc.display()
    );
    run_lua(&script).await.unwrap();
}
