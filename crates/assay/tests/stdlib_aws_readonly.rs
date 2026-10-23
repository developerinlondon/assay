//! What the `assay.aws.*` modules can and cannot do under read-only mode.
//!
//! Readonly classifies an AWS request by what it asks for, not by its verb, so
//! the modules work with no policy file at all: resolving credentials through
//! IRSA or an assumed role is a `POST` that only exchanges a token, and ECR's
//! `GetAuthorizationToken` is a `POST` that only mints a short-lived registry
//! password. Both are reads, and both now run.
//!
//! The same classification closes the hole the other way round. The EC2 Query
//! API takes its action in the query string, so a signed
//! `GET ?Action=TerminateInstances` used to pass readonly on its verb alone.
//! Anything outside the read allowlist is refused whatever the verb.
//!
//! The classifier is strict about the host, so these tests use the real AWS
//! hostnames and point them at the mock with DNS overrides rather than talking
//! to a loopback address an AWS signature would never legitimately name. The
//! host has to back the service the signature claims, too — otherwise the
//! exemption would reach every mutating endpoint AWS hosts for its customers.
//!
//! An operator policy with `http.rules` still owns classification outright,
//! default-deny included; the tests at the end pin that the built-in stands
//! aside when one is loaded.
//!
//! Each test builds its own VM on purpose: `sts.lua` caches resolved
//! credentials in a module-level table, so a warm entry leaking between tests
//! would let a successful exchange mask a block in the next one.

mod common;

use assay::lua::ExecMode;
use common::aws::{WebIdentityProfile, sts_json, web_identity_profile};
use common::aws_vm::*;
use wiremock::matchers::{body_string_contains, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A rule granting `classify: read` to one POST target — the way an operator
/// says "this exact target is a read" when they have taken classification
/// over with `http.rules`.
fn read_rule(host: &str, request_path: &str) -> String {
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
fn write_rule(host: &str, request_path: &str) -> String {
    format!(
        r#"    - hosts: ["{host}"]
      methods: [POST]
      paths: ["{request_path}"]
"#
    )
}

/// Sign a request the way a script reaching past the `assay.aws.*` modules
/// would, then send it. Everything the classifier reads — the credential
/// scope, the action — comes out of a real signature.
fn signed_get_script(service: &str, host: &str, url: &str, query: &str) -> String {
    format!(
        r#"
        local sigv4 = require("assay.aws.sigv4")
        local query = "{query}"
        local headers = sigv4.sign({{
            access_key = "{ACCESS_KEY}",
            secret_key = "{SECRET_KEY}",
            service = "{service}",
            region = "us-east-1",
            method = "GET",
            host = "{host}",
            path = "/",
            query = query,
            payload = "",
        }})
        http.get("{url}/?" .. query, {{ headers = headers }})
        return "reached"
        "#
    )
}

fn signed_post_script(service: &str, host: &str, url: &str, body: &str) -> String {
    format!(
        r#"
        local sigv4 = require("assay.aws.sigv4")
        local body = "{body}"
        local headers = sigv4.sign({{
            access_key = "{ACCESS_KEY}",
            secret_key = "{SECRET_KEY}",
            service = "{service}",
            region = "us-east-1",
            method = "POST",
            host = "{host}",
            path = "/",
            payload = body,
            headers = {{ ["content-type"] = "application/x-www-form-urlencoded" }},
        }})
        http.post("{url}/", body, {{ headers = headers }})
        return "reached"
        "#
    )
}

/// Resolve through the web-identity profile and return the access key it
/// produced. No explicit keys, so this is the IRSA path.
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
        .and(path("/"))
        .and(body_string_contains("Action=AssumeRoleWithWebIdentity"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(sts_json("AssumeRoleWithWebIdentity", "ASIAFROMFILE")),
        )
        .mount(server)
        .await;
}

const DESCRIBE_INSTANCES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
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

async fn mount_describe_instances(server: &MockServer) {
    Mock::given(method("GET"))
        .and(query_param("Action", "DescribeInstances"))
        .respond_with(ResponseTemplate::new(200).set_body_string(DESCRIBE_INSTANCES_XML))
        .mount(server)
        .await;
}

async fn mount_ecr_token(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"authorizationData":[{"authorizationToken":"QVdTOnBhc3N3b3Jk","proxyEndpoint":"https://123456789012.dkr.ecr.us-east-1.amazonaws.com","expiresAt":"2026-01-01T00:00:00Z"}]}"#,
        ))
        .mount(server)
        .await;
}

fn ecr_token_script(endpoint: &str) -> String {
    format!(
        r#"
        local ecr = require("assay.aws.ecr")
        local c = ecr.client({{
            access_key = "{ACCESS_KEY}",
            secret_key = "{SECRET_KEY}",
            region = "us-east-1",
            endpoint = "{endpoint}",
        }})
        return c:get_authorization_token().token
        "#
    )
}

// ------------------------------------------------- the reads readonly allows

/// The reported bug, now the other way round: an IRSA-authenticated script
/// gets its credentials and reads EC2 under readonly with no policy at all.
/// Two gated hops in one script — the unsigned `AssumeRoleWithWebIdentity`
/// POST that `ec2.client` makes on the way to its first request, then the
/// signed `DescribeInstances` GET.
#[tokio::test]
async fn irsa_credentials_and_an_ec2_read_work_under_readonly_without_a_policy() {
    let server = MockServer::start().await;
    mount_web_identity_sts(&server).await;
    mount_describe_instances(&server).await;
    let profile = web_identity_profile();

    let vm = readonly_vm(&server, None);
    let out = eval(
        &vm,
        &format!(
            r#"
            local ec2 = require("assay.aws.ec2")
            local c = ec2.client({{
                profile = "irsa",
                credentials_file = "/nonexistent-credentials",
                config_file = "{config}",
                sts_endpoint = "{sts}",
                region = "us-east-1",
                endpoint = "{ec2}",
            }})
            local instances = c:describe_instances()
            assert.eq(#instances, 1)
            return instances[1].instance_id
            "#,
            config = profile.config_path.display(),
            sts = endpoint(STS_HOST, &server),
            ec2 = endpoint(EC2_HOST, &server),
        ),
    )
    .await
    .unwrap();

    assert_eq!(out, "i-readonly");
    // Both hops really happened: the token exchange, then the describe.
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2, "expected the exchange and the describe");
}

/// Minting a registry password changes nothing, so ECR's signed `POST` is a
/// read too — with static keys there is not even an STS hop involved.
#[tokio::test]
async fn ecr_get_authorization_token_works_under_readonly_without_a_policy() {
    let server = MockServer::start().await;
    mount_ecr_token(&server).await;

    let vm = readonly_vm(&server, None);
    let token = eval(&vm, &ecr_token_script(&endpoint(ECR_HOST, &server)))
        .await
        .unwrap();

    // The module base64-decodes the AWS:<password> pair ECR returns.
    assert_eq!(token, "password");
}

/// EC2's Query API is signed as a `GET`, and with credentials already in hand
/// its reads need no policy. This is the claim `docs/modules/aws-ec2.md` makes.
#[tokio::test]
async fn ec2_describe_instances_works_under_readonly_with_static_credentials() {
    let server = MockServer::start().await;
    mount_describe_instances(&server).await;

    let vm = readonly_vm(&server, None);
    let out = eval(
        &vm,
        &format!(
            r#"
            local ec2 = require("assay.aws.ec2")
            local c = ec2.client({{
                access_key = "{ACCESS_KEY}",
                secret_key = "{SECRET_KEY}",
                region = "us-east-1",
                endpoint = "{ec2}",
            }})
            local instances = c:describe_instances()
            assert.eq(#instances, 1)
            return instances[1].instance_id
            "#,
            ec2 = endpoint(EC2_HOST, &server),
        ),
    )
    .await
    .unwrap();

    assert_eq!(out, "i-readonly");
}

/// `amazonaws.com` hosts far more than AWS's own APIs, and an API Gateway
/// route may perfectly well take a query parameter called `action`. Nothing
/// about that is an AWS API call, so readonly leaves it to the verb and this
/// `GET` runs as it always did.
#[tokio::test]
async fn an_unsigned_get_to_a_non_api_aws_host_still_runs_under_readonly() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(query_param("action", "ping"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pong"))
        .mount(&server)
        .await;
    let url = endpoint(GATEWAY_HOST, &server);

    let vm = readonly_vm(&server, None);
    let script = format!(r#"return http.get("{url}/prod/status?action=ping").body"#);
    let out = eval(&vm, &script).await.unwrap();

    assert_eq!(out, "pong");
}

/// Approval mode consumes the same classifier, so neither hop prompts. With no
/// approved indices configured, a gated call would raise an approval request
/// and the script would never return a value.
#[tokio::test]
async fn an_aws_read_does_not_suspend_under_approval_mode() {
    let server = MockServer::start().await;
    mount_web_identity_sts(&server).await;
    mount_describe_instances(&server).await;
    let profile = web_identity_profile();

    let vm = vm_in_mode(&server, ExecMode::Approval, None);
    let out = eval(
        &vm,
        &format!(
            r#"
            local ec2 = require("assay.aws.ec2")
            local c = ec2.client({{
                profile = "irsa",
                credentials_file = "/nonexistent-credentials",
                config_file = "{config}",
                sts_endpoint = "{sts}",
                region = "us-east-1",
                endpoint = "{ec2}",
            }})
            return c:describe_instances()[1].instance_id
            "#,
            config = profile.config_path.display(),
            sts = endpoint(STS_HOST, &server),
            ec2 = endpoint(EC2_HOST, &server),
        ),
    )
    .await
    .unwrap();

    assert_eq!(out, "i-readonly");
}

// ------------------------------------------------ what readonly still refuses

/// A mutating POST to a service readonly otherwise admits reads on.
#[tokio::test]
async fn a_signed_post_running_instances_is_refused_under_readonly() {
    let server = MockServer::start().await;
    let url = endpoint(EC2_HOST, &server);
    let host = signed_host(EC2_HOST, &server);
    let body = "Action=RunInstances&ImageId=ami-1&Version=2016-11-15";
    let script = signed_post_script("ec2", &host, &url, body);

    let vm = readonly_vm(&server, None);
    let err = eval_err(&vm, &script).await;

    assert!(err.contains("readonly: http.post blocked"), "got: {err}");
    let why = "aws ec2 Action=RunInstances is not a read";
    assert!(err.contains(why), "got: {err}");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "the call should be refused before the socket is opened"
    );
}

/// The hole this closes: the Query API takes its action in the query string,
/// so classifying by verb let a termination through on a `GET`.
#[tokio::test]
async fn a_signed_get_terminating_instances_is_refused_under_readonly() {
    let server = MockServer::start().await;
    let url = endpoint(EC2_HOST, &server);
    let host = signed_host(EC2_HOST, &server);
    let query = "Action=TerminateInstances&InstanceId.1=i-1&Version=2016-11-15";
    let script = signed_get_script("ec2", &host, &url, query);

    let vm = readonly_vm(&server, None);
    let err = eval_err(&vm, &script).await;

    assert!(err.contains("readonly: http.get blocked"), "got: {err}");
    let why = "aws ec2 Action=TerminateInstances is not a read";
    assert!(err.contains(why), "got: {err}");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "the call should be refused before the socket is opened"
    );
}

/// `GetSessionToken` mints credentials for someone else to use, so it is not a
/// read however much its name looks like one — the reason the allowlist is
/// enumerated rather than a `Get*` prefix rule.
#[tokio::test]
async fn minting_a_session_token_is_refused_under_readonly() {
    let server = MockServer::start().await;
    let url = endpoint(STS_HOST, &server);
    let host = signed_host(STS_HOST, &server);
    let body = "Action=GetSessionToken&Version=2011-06-15";
    let script = signed_post_script("sts", &host, &url, body);

    let vm = readonly_vm(&server, None);
    let err = eval_err(&vm, &script).await;

    let why = "aws sts Action=GetSessionToken is not a read";
    assert!(err.contains(why), "got: {err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// Two `Action` parameters: which one the service honours is its business, not
/// something to guess at while deciding whether to admit the call.
#[tokio::test]
async fn a_duplicate_action_is_refused_under_readonly() {
    let server = MockServer::start().await;
    let url = endpoint(EC2_HOST, &server);
    let host = signed_host(EC2_HOST, &server);
    let query = "Action=DescribeInstances&Action=TerminateInstances";
    let script = signed_get_script("ec2", &host, &url, query);

    let vm = readonly_vm(&server, None);
    let err = eval_err(&vm, &script).await;

    let why = "aws request carries more than one Action";
    assert!(err.contains(why), "got: {err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// A real signature plus a second spelling of `Authorization`. The transport
/// lowercases header names and sends one value per name, so the classifier
/// must not read one value while AWS receives the other. `table_order` is the
/// Lua constructor listing both entries, which sets the iteration order.
fn colliding_header_script(host: &str, url: &str, query: &str, table_order: &str) -> String {
    format!(
        r#"
        local sigv4 = require("assay.aws.sigv4")
        local query = "{query}"
        local signed = sigv4.sign({{
            access_key = "{ACCESS_KEY}",
            secret_key = "{SECRET_KEY}",
            service = "ec2",
            region = "us-east-1",
            method = "GET",
            host = "{host}",
            path = "/",
            query = query,
            payload = "",
        }})
        local real = signed.authorization
        signed.authorization = nil
        local headers = {table_order}
        for name, value in pairs(signed) do headers[name] = value end
        http.get("{url}/?" .. query, {{ headers = headers }})
        return "reached"
        "#
    )
}

#[tokio::test]
async fn a_header_named_twice_in_different_case_is_refused_under_readonly() {
    let orders = [
        r#"{ ["Authorization"] = "Bearer x", ["authorization"] = real }"#,
        r#"{ ["authorization"] = real, ["Authorization"] = "Bearer x" }"#,
        r#"{ ["AUTHORIZATION"] = real, ["authorization"] = "Bearer x" }"#,
        r#"{ ["authorization"] = "Bearer x", ["AUTHORIZATION"] = real }"#,
    ];
    for order in orders {
        let server = MockServer::start().await;
        let url = endpoint(EC2_HOST, &server);
        let host = signed_host(EC2_HOST, &server);
        let query = "Action=TerminateInstances&InstanceId.1=i-1&Version=2016-11-15";
        let script = colliding_header_script(&host, &url, query, order);

        let vm = readonly_vm(&server, None);
        let err = eval_err(&vm, &script).await;

        assert!(err.contains("readonly: http.get blocked"), "{order}: {err}");
        assert!(
            err.contains("aws request names a header twice"),
            "{order}: {err}"
        );
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "{order}: the call should be refused before the socket is opened"
        );
    }
}

/// `Action` and `action` are one parameter to the classifier, so a case
/// variant cannot smuggle a second action past it.
#[tokio::test]
async fn an_action_repeated_in_different_case_is_refused_under_readonly() {
    let server = MockServer::start().await;
    let url = endpoint(EC2_HOST, &server);
    let host = signed_host(EC2_HOST, &server);
    let query = "Action=DescribeInstances&action=TerminateInstances";
    let script = signed_get_script("ec2", &host, &url, query);

    let vm = readonly_vm(&server, None);
    let err = eval_err(&vm, &script).await;

    assert!(
        err.contains("aws request carries more than one Action"),
        "got: {err}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// A signature is a claim about a service, not a capability. Claiming `sts` on
/// an AWS host STS does not own — an API Gateway stage, which serves whatever
/// the customer deployed and never verifies SigV4 — must buy nothing, or the
/// read exemption reaches every mutating endpoint under `amazonaws.com`.
#[tokio::test]
async fn a_service_claim_the_host_does_not_back_is_refused_under_readonly() {
    let server = MockServer::start().await;
    let url = endpoint(GATEWAY_HOST, &server);
    let host = signed_host(GATEWAY_HOST, &server);
    let body = "Action=AssumeRole&Version=2011-06-15";
    let script = signed_post_script("sts", &host, &url, body);

    let vm = readonly_vm(&server, None);
    let err = eval_err(&vm, &script).await;

    assert!(err.contains("readonly: http.post blocked"), "got: {err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// And a signature on a host outside AWS entirely is just a header.
#[tokio::test]
async fn a_forged_signature_to_a_host_outside_aws_is_refused_under_readonly() {
    let server = MockServer::start().await;
    let url = endpoint(OUTSIDE_HOST, &server);
    let host = signed_host(OUTSIDE_HOST, &server);
    let body = "Action=AssumeRole&Version=2011-06-15";
    let script = signed_post_script("sts", &host, &url, body);

    let vm = readonly_vm(&server, None);
    let err = eval_err(&vm, &script).await;

    assert!(err.contains("readonly: http.post blocked"), "got: {err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// And a host that merely ends in the right letters is not an AWS host: the
/// suffix has to start at a label boundary.
#[tokio::test]
async fn a_forged_signature_to_a_lookalike_host_is_refused_under_readonly() {
    let server = MockServer::start().await;
    let url = endpoint(LOOKALIKE_HOST, &server);
    let host = signed_host(LOOKALIKE_HOST, &server);
    let body = "Action=AssumeRole&Version=2011-06-15";
    let script = signed_post_script("sts", &host, &url, body);

    let vm = readonly_vm(&server, None);
    let err = eval_err(&vm, &script).await;

    assert!(err.contains("readonly: http.post blocked"), "got: {err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

// --------------------------------------------- a policy owns classification

/// A rule without `classify:` only admits its target; it says nothing about
/// what the request does, so the built-in still does. The exchange is a read
/// to the built-in, and runs.
#[tokio::test]
async fn a_rule_without_classify_leaves_the_built_in_in_charge() {
    let server = MockServer::start().await;
    mount_web_identity_sts(&server).await;
    let profile = web_identity_profile();

    let policy_yaml = policy(&[write_rule(STS_HOST, "/")]);
    let script = resolve_credentials_script(&profile, &endpoint(STS_HOST, &server));
    let vm = readonly_vm(&server, Some(&policy_yaml));
    let access_key = eval(&vm, &script).await.unwrap();

    assert_eq!(access_key, "ASIAFROMFILE");
}

fn get_rule(host: &str, classify: &str) -> String {
    format!("    - hosts: [\"{host}\"]\n      methods: [GET]\n{classify}")
}

/// The natural rule for an EC2 reader — this host, `GET` — must not reopen the
/// mutating `GET` the built-in refuses.
#[tokio::test]
async fn a_get_rule_for_ec2_does_not_reopen_a_mutating_get() {
    let server = MockServer::start().await;
    let url = endpoint(EC2_HOST, &server);
    let host = signed_host(EC2_HOST, &server);
    let query = "Action=TerminateInstances&InstanceId.1=i-1&Version=2016-11-15";
    let script = signed_get_script("ec2", &host, &url, query);

    let policy_yaml = policy(&[get_rule(EC2_HOST, "")]);
    let vm = readonly_vm(&server, Some(&policy_yaml));
    let err = eval_err(&vm, &script).await;

    let why = "aws ec2 Action=TerminateInstances is not a read";
    assert!(err.contains(why), "got: {err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// `classify: read` is the operator's explicit word, and it overrides the
/// built-in — the way to admit an AWS read the allowlist does not name.
#[tokio::test]
async fn an_explicit_read_rule_overrides_the_built_in() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(query_param("Action", "DescribeStacks"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<Stacks/>"))
        .mount(&server)
        .await;
    let url = endpoint(EC2_HOST, &server);
    let host = signed_host(EC2_HOST, &server);
    let script = signed_get_script("ec2", &host, &url, "Action=DescribeStacks");

    let policy_yaml = policy(&[get_rule(EC2_HOST, "      classify: read\n")]);
    let vm = readonly_vm(&server, Some(&policy_yaml));
    assert_eq!(eval(&vm, &script).await.unwrap(), "reached");
}

/// With classification in the policy's hands, `classify: read` is how the same
/// exchange is admitted — unchanged behaviour, and still the mechanism for any
/// host the built-in says nothing about.
#[tokio::test]
async fn a_read_rule_admits_the_exchange_when_a_policy_defines_rules() {
    let server = MockServer::start().await;
    mount_web_identity_sts(&server).await;
    let profile = web_identity_profile();

    let policy_yaml = policy(&[read_rule(STS_HOST, "/")]);
    let script = resolve_credentials_script(&profile, &endpoint(STS_HOST, &server));
    let vm = readonly_vm(&server, Some(&policy_yaml));
    let access_key = eval(&vm, &script).await.unwrap();

    assert_eq!(access_key, "ASIAFROMFILE");
}

/// The exemption is scoped to what it names. A read rule for STS must not turn
/// the rest of the account into reads — including EC2, which the same script
/// can reach. The second rule admits the EC2 POST at the policy layer
/// precisely so that what refuses it is readonly's own gate, not a
/// no-rule-matched denial.
#[tokio::test]
async fn a_read_rule_for_sts_does_not_admit_a_post_to_ec2() {
    let server = MockServer::start().await;
    mount_web_identity_sts(&server).await;
    Mock::given(method("POST"))
        .and(body_string_contains("Action=RunInstances"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<Response/>"))
        .mount(&server)
        .await;
    let profile = web_identity_profile();

    let policy_yaml = policy(&[read_rule(STS_HOST, "/"), write_rule(EC2_HOST, "/")]);
    let vm = readonly_vm(&server, Some(&policy_yaml));
    let err = eval(
        &vm,
        &format!(
            r#"
            local sts = require("assay.aws.sts")
            local c = sts.credentials({{
                profile = "irsa",
                credentials_file = "/nonexistent-credentials",
                config_file = "{config}",
                sts_endpoint = "{sts}",
            }})
            assert.eq(c.access_key, "ASIAFROMFILE")

            -- A target the policy allows but does not reclassify. The STS
            -- exemption must not carry over to it.
            http.post("{ec2}/", "Action=RunInstances")
            return "reached"
            "#,
            config = profile.config_path.display(),
            sts = endpoint(STS_HOST, &server),
            ec2 = endpoint(EC2_HOST, &server),
        ),
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(err.contains("readonly: http.post blocked"), "got: {err}");
    // Exactly one request proves both halves at once: the STS exchange ran, and
    // the EC2 POST was stopped before the socket was opened.
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "the EC2 POST should not have been sent");
    assert!(
        String::from_utf8_lossy(&requests[0].body).contains("AssumeRoleWithWebIdentity"),
        "the one request should be the token exchange"
    );
}
