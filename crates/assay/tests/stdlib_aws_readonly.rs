//! What the `assay.aws.*` modules can and cannot do under read-only mode.
//!
//! The modules split cleanly in two, and the split is not the one the module
//! names suggest. EC2 and S3 sign the Query API as `GET`, so their reads run
//! under readonly untouched — but only once a client exists, and building one
//! without explicit keys resolves credentials through `assay.aws.sts`, whose
//! IRSA and assume-role paths are `POST`s. ECR is the same shape one level up:
//! `GetAuthorizationToken` mints a short-lived password and changes nothing,
//! yet travels as a signed `POST`.
//!
//! Readonly classifies by verb, so all three of those reads are refused by
//! default. `classify: read` in a policy file is the documented way to say
//! "this exact target is a read"; these tests pin both halves — refused with no
//! policy, admitted with one — and that the exemption does not spread beyond
//! the host and path it names.
//!
//! Each test builds its own VM on purpose: `sts.lua` caches resolved
//! credentials in a module-level table, so a warm entry leaking between tests
//! would let a successful exchange mask a block in the next one.

mod common;

use std::sync::Arc;

use assay::lua::policy::Policy;
use assay::lua::{ExecMode, VmOptions, create_vm_with_policy};
use common::aws::{WebIdentityProfile, sts_json, web_identity_profile};
use wiremock::matchers::{body_string_contains, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap()
}

fn readonly_vm(policy_yaml: Option<&str>) -> mlua::Lua {
    let parsed = policy_yaml.map(|yaml| Arc::new(Policy::parse(yaml).expect("policy parses")));
    create_vm_with_policy(
        client(),
        VmOptions {
            mode: ExecMode::ReadOnly,
            ..Default::default()
        },
        parsed,
    )
    .unwrap()
}

async fn eval(vm: &mlua::Lua, script: &str) -> mlua::Result<String> {
    vm.load(script).eval_async::<String>().await
}

/// A rule granting `classify: read` to one POST target on the mock host — the
/// documented way to say "this exact target is a read".
fn read_rule(server: &MockServer, request_path: &str) -> String {
    let host = server.address().ip();
    format!(
        r#"    - hosts: ["{host}"]
      methods: [POST]
      paths: ["{request_path}"]
      classify: read
"#
    )
}

/// A rule that admits a POST without reclassifying it. The policy allows the
/// target, so a POST matching it defaults to a write and whether it runs is
/// left to the mode's own gate — which is what lets a test tell "readonly
/// refused this" apart from "no rule matched it".
fn write_rule(server: &MockServer, request_path: &str) -> String {
    let host = server.address().ip();
    format!(
        r#"    - hosts: ["{host}"]
      methods: [POST]
      paths: ["{request_path}"]
"#
    )
}

fn policy(rules: &[String]) -> String {
    format!("version: 1\nhttp:\n  rules:\n{}", rules.concat())
}

/// Resolve via that profile and return the access key it produced.
fn resolve_credentials_script(profile: &WebIdentityProfile, sts_endpoint: &str) -> String {
    format!(
        r#"
        local sts = require("assay.aws.sts")
        local c = sts.credentials({{
            profile = "irsa",
            credentials_file = "/nonexistent-credentials",
            config_file = "{config}",
            sts_endpoint = "{sts}",
        }})
        return c.access_key
        "#,
        config = profile.config_path.display(),
        sts = sts_endpoint
    )
}

async fn mount_web_identity_sts(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/sts"))
        .and(body_string_contains("Action=AssumeRoleWithWebIdentity"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(sts_json("AssumeRoleWithWebIdentity", "ASIAFROMFILE")),
        )
        .mount(server)
        .await;
}

// ------------------------------------------------- STS credential resolution

/// The reported bug: an IRSA-authenticated script cannot get credentials at
/// all, so every `assay.aws.*` client built on them fails before its first
/// request.
#[tokio::test]
async fn irsa_credential_resolution_is_blocked_under_readonly_without_a_policy() {
    let server = MockServer::start().await;
    mount_web_identity_sts(&server).await;
    let profile = web_identity_profile();

    let vm = readonly_vm(None);
    let err = eval(
        &vm,
        &resolve_credentials_script(&profile, &format!("{}/sts", server.uri())),
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(err.contains("readonly: http.post blocked"), "got: {err}");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "the exchange should be refused before the socket is opened"
    );
}

/// And the fix available today: `classify: read` on the STS endpoint admits the
/// exchange, so the chain completes under readonly.
#[tokio::test]
async fn irsa_credential_resolution_proceeds_under_readonly_with_a_read_rule() {
    let server = MockServer::start().await;
    mount_web_identity_sts(&server).await;
    let profile = web_identity_profile();

    let policy_yaml = policy(&[read_rule(&server, "/sts")]);
    let vm = readonly_vm(Some(&policy_yaml));
    let access_key = eval(
        &vm,
        &resolve_credentials_script(&profile, &format!("{}/sts", server.uri())),
    )
    .await
    .unwrap();

    assert_eq!(access_key, "ASIAFROMFILE");
}

/// The exemption is scoped to what it names. A read rule for STS must not turn
/// the rest of the account into reads — including the EC2 endpoint, which the
/// same script can reach. The second rule admits the EC2 POST at the policy
/// layer precisely so that what refuses it is readonly's own gate, not a
/// no-rule-matched denial.
#[tokio::test]
async fn a_read_rule_for_sts_does_not_admit_a_post_to_ec2() {
    let server = MockServer::start().await;
    mount_web_identity_sts(&server).await;
    Mock::given(method("POST"))
        .and(path("/ec2/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<Response/>"))
        .mount(&server)
        .await;
    let profile = web_identity_profile();

    let policy_yaml = policy(&[read_rule(&server, "/sts"), write_rule(&server, "/ec2/")]);
    let vm = readonly_vm(Some(&policy_yaml));
    let err = eval(
        &vm,
        &format!(
            r#"
            local sts = require("assay.aws.sts")
            local c = sts.credentials({{
                profile = "irsa",
                credentials_file = "/nonexistent-credentials",
                config_file = "{config}",
                sts_endpoint = "{uri}/sts",
            }})
            assert.eq(c.access_key, "ASIAFROMFILE")

            -- Same host, same verb, a path the policy allows but does not
            -- reclassify. The STS exemption must not carry over to it.
            http.post("{uri}/ec2/", "Action=RunInstances")
            return "reached"
            "#,
            config = profile.config_path.display(),
            uri = server.uri()
        ),
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(err.contains("readonly: http.post blocked"), "got: {err}");
    // Exactly one request proves both halves at once: the STS exchange ran, and
    // the EC2 POST was stopped before the socket was opened.
    let requests = server.received_requests().await.unwrap();
    let paths: Vec<_> = requests.iter().map(|r| r.url.path()).collect();
    assert_eq!(paths, vec!["/sts"], "unexpected requests: {paths:?}");
}

// ------------------------------------------------------------ EC2 Query reads

/// EC2's Query API is signed as a `GET`, so with credentials already in hand
/// its reads need no policy at all. This is the claim `docs/modules/aws-ec2.md`
/// makes; nothing pinned it before.
#[tokio::test]
async fn ec2_describe_instances_works_under_readonly_with_static_credentials() {
    let server = MockServer::start().await;
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
  <reservationSet>
    <item>
      <reservationId>r-1</reservationId>
      <instancesSet>
        <item>
          <instanceId>i-readonly</instanceId>
          <instanceType>t3.micro</instanceType>
          <instanceState><code>16</code><name>running</name></instanceState>
          <privateIpAddress>10.0.0.10</privateIpAddress>
          <placement><availabilityZone>us-east-1a</availabilityZone></placement>
        </item>
      </instancesSet>
    </item>
  </reservationSet>
</DescribeInstancesResponse>"#;
    Mock::given(method("GET"))
        .and(path("/"))
        .and(query_param("Action", "DescribeInstances"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let vm = readonly_vm(None);
    let out = eval(
        &vm,
        &format!(
            r#"
            local ec2 = require("assay.aws.ec2")
            local c = ec2.client({{
                access_key = "AKIAIOSFODNN7EXAMPLE",
                secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
                region = "us-east-1",
                endpoint = "{}",
            }})
            local instances = c:describe_instances()
            assert.eq(#instances, 1)
            return instances[1].instance_id
            "#,
            server.uri()
        ),
    )
    .await
    .unwrap();

    assert_eq!(out, "i-readonly");
}

// ------------------------------------------------------ ECR authorization token

fn ecr_token_script(endpoint: &str) -> String {
    format!(
        r#"
        local ecr = require("assay.aws.ecr")
        local c = ecr.client({{
            access_key = "AKIAIOSFODNN7EXAMPLE",
            secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            region = "us-east-1",
            endpoint = "{endpoint}",
        }})
        return c:get_authorization_token().token
        "#
    )
}

async fn mount_ecr_token(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/ecr/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"authorizationData":[{"authorizationToken":"QVdTOnBhc3N3b3Jk","proxyEndpoint":"https://123456789012.dkr.ecr.us-east-1.amazonaws.com","expiresAt":"2026-01-01T00:00:00Z"}]}"#,
        ))
        .mount(server)
        .await;
}

/// Minting a registry password changes nothing, but ECR's JSON protocol sends
/// it as a POST — so readonly refuses it even with static credentials, where
/// no STS hop is involved at all.
#[tokio::test]
async fn ecr_get_authorization_token_is_blocked_under_readonly_without_a_policy() {
    let server = MockServer::start().await;
    mount_ecr_token(&server).await;

    let vm = readonly_vm(None);
    let err = eval(&vm, &ecr_token_script(&format!("{}/ecr", server.uri())))
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("readonly: http.post blocked"), "got: {err}");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "the mint should be refused before the socket is opened"
    );
}

#[tokio::test]
async fn ecr_get_authorization_token_proceeds_under_readonly_with_a_read_rule() {
    let server = MockServer::start().await;
    mount_ecr_token(&server).await;

    let policy_yaml = policy(&[read_rule(&server, "/ecr/")]);
    let vm = readonly_vm(Some(&policy_yaml));
    let token = eval(&vm, &ecr_token_script(&format!("{}/ecr", server.uri())))
        .await
        .unwrap();

    // The module base64-decodes the AWS:<password> pair ECR returns.
    assert_eq!(token, "password");
}
