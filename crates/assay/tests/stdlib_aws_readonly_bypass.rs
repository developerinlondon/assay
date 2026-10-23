//! Readonly and approval against requests whose URL alone does not say what
//! they reach: signatures that do not read cleanly, a `Host` header, an IP
//! literal, a redirect. Each case is a bypass that once let a signed
//! `TerminateInstances` through, replayed against wiremock.

mod common;

use assay::lua::ExecMode;
use common::aws_vm::*;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ------------------------------------------- what the URL alone does not say

const TERMINATE_QUERY: &str = "Action=TerminateInstances&InstanceId.1=i-1&Version=2016-11-15";
const SCOPE: &str = "AKIAIOSFODNN7EXAMPLE/20260101/us-east-1/ec2/aws4_request";

/// `http.get` of `url` with `headers` given as a Lua table constructor.
fn get_with_headers(url: &str, headers: &str) -> String {
    format!(
        r#"
        http.get("{url}", {{ headers = {headers} }})
        return "reached"
        "#
    )
}

async fn refused_before_sending(server: &MockServer, script: &str, why: &str) {
    let vm = readonly_vm(server, None);
    let err = eval_err(&vm, script).await;
    assert!(err.contains(why), "want {why:?}, got: {err}");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "the call should be refused before the socket is opened"
    );
}

/// The algorithm name is case-insensitive to AWS, so it is to the classifier.
#[tokio::test]
async fn a_lowercase_algorithm_is_still_read_as_a_signature() {
    let server = MockServer::start().await;
    let url = format!("{}/?{TERMINATE_QUERY}", endpoint(EC2_HOST, &server));
    let auth = format!("aws4-hmac-sha256 Credential={SCOPE}, SignedHeaders=host, Signature=ab");
    let script = get_with_headers(&url, &format!("{{ authorization = \"{auth}\" }}"));
    refused_before_sending(
        &server,
        &script,
        "aws ec2 Action=TerminateInstances is not a read",
    )
    .await;
}

/// The same through an `http.client` handle, which has its own gate.
#[tokio::test]
async fn a_lowercase_algorithm_through_a_client_handle_is_refused() {
    let server = MockServer::start().await;
    let url = format!("{}/?{TERMINATE_QUERY}", endpoint(EC2_HOST, &server));
    let auth = format!("aws4-hmac-sha256 Credential={SCOPE}, SignedHeaders=host, Signature=ab");
    let script = format!(
        r#"
        local c = http.client({{ timeout = 2 }})
        c:get("{url}", {{ headers = {{ authorization = "{auth}" }} }})
        return "reached"
        "#
    );
    refused_before_sending(
        &server,
        &script,
        "aws ec2 Action=TerminateInstances is not a read",
    )
    .await;
}

#[tokio::test]
async fn a_decoy_credential_component_is_refused() {
    let server = MockServer::start().await;
    let url = format!("{}/?{TERMINATE_QUERY}", endpoint(EC2_HOST, &server));
    let auth = format!(
        "AWS4-HMAC-SHA256 Foo=Credential=x, Credential={SCOPE}, SignedHeaders=host, Signature=ab"
    );
    let script = get_with_headers(&url, &format!("{{ authorization = \"{auth}\" }}"));
    refused_before_sending(&server, &script, "a signature that cannot be read").await;
}

#[tokio::test]
async fn sigv2_query_authentication_is_refused() {
    let server = MockServer::start().await;
    let sigv2 = "AWSAccessKeyId=AKIA&SignatureVersion=2&SignatureMethod=HmacSHA256&Signature=ab";
    let url = format!("{}/?{TERMINATE_QUERY}&{sigv2}", endpoint(EC2_HOST, &server));
    let script = get_with_headers(&url, "{}");
    refused_before_sending(&server, &script, "a signature that cannot be read").await;
}

#[tokio::test]
async fn a_presigned_termination_is_refused() {
    let server = MockServer::start().await;
    let presign = "X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AKIA%2F20260101%2Fus-east-1%2Fec2%2Faws4_request&X-Amz-Signature=ab";
    let url = format!(
        "{}/?{TERMINATE_QUERY}&{presign}",
        endpoint(EC2_HOST, &server)
    );
    let script = get_with_headers(&url, "{}");
    refused_before_sending(
        &server,
        &script,
        "aws ec2 Action=TerminateInstances is not a read",
    )
    .await;
}

/// An IP-literal URL is not an AWS host, but a caller-set `Host` header is
/// what the server routes on.
#[tokio::test]
async fn a_host_header_naming_another_host_is_refused() {
    let server = MockServer::start().await;
    let url = format!("{}/?{TERMINATE_QUERY}", server.uri());
    let auth = format!("AWS4-HMAC-SHA256 Credential={SCOPE}, SignedHeaders=host, Signature=ab");
    let headers = format!("{{ host = \"{EC2_HOST}\", authorization = \"{auth}\" }}");
    let script = get_with_headers(&url, &headers);
    refused_before_sending(&server, &script, "Host header does not name the URL's host").await;
}

/// A redirect to a presigned termination, from a host that is not AWS at all.
async fn mount_redirect_to_termination(server: &MockServer) -> String {
    let presign =
        "X-Amz-Credential=AKIA%2F20260101%2Fus-east-1%2Fec2%2Faws4_request&X-Amz-Signature=ab";
    let target = format!(
        "{}/?{TERMINATE_QUERY}&{presign}",
        endpoint(EC2_HOST, server)
    );
    Mock::given(method("GET"))
        .and(path("/r"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", target.as_str()))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(query_param("Action", "TerminateInstances"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<terminated/>"))
        .mount(server)
        .await;
    format!("{}/r", endpoint(OUTSIDE_HOST, server))
}

async fn assert_only_the_redirect_was_sent(server: &MockServer) {
    let requests = server.received_requests().await.unwrap();
    let paths: Vec<_> = requests.iter().map(|r| r.url.to_string()).collect();
    assert_eq!(requests.len(), 1, "the redirect target was sent: {paths:?}");
    assert_eq!(requests[0].url.path(), "/r");
}

#[tokio::test]
async fn a_redirect_to_a_mutation_is_refused_under_readonly() {
    let server = MockServer::start().await;
    let start = mount_redirect_to_termination(&server).await;

    let vm = readonly_vm(&server, None);
    let err = eval_err(&vm, &get_with_headers(&start, "{}")).await;

    let why = "readonly: redirect to ec2.us-east-1.amazonaws.com blocked";
    assert!(err.contains(why), "got: {err}");
    assert!(
        err.contains("Action=TerminateInstances is not a read"),
        "got: {err}"
    );
    assert_only_the_redirect_was_sent(&server).await;
}

#[tokio::test]
async fn a_redirect_to_a_mutation_is_refused_under_approval() {
    let server = MockServer::start().await;
    let start = mount_redirect_to_termination(&server).await;

    let vm = vm_in_mode(&server, ExecMode::Approval, None);
    let err = eval_err(&vm, &get_with_headers(&start, "{}")).await;

    assert!(
        err.contains("approval: redirect to ec2.us-east-1.amazonaws.com blocked"),
        "got: {err}"
    );
    assert_only_the_redirect_was_sent(&server).await;
}

/// A policy is re-checked on every hop too, whatever the mode.
#[tokio::test]
async fn a_redirect_off_the_policy_is_refused() {
    let server = MockServer::start().await;
    let start = mount_redirect_to_termination(&server).await;

    let rule = format!("    - hosts: [\"{OUTSIDE_HOST}\"]\n");
    let vm = vm_in_mode(&server, ExecMode::Unrestricted, Some(&policy(&[rule])));
    let err = eval_err(&vm, &get_with_headers(&start, "{}")).await;

    assert!(
        err.contains("is not allowed by any http rule"),
        "got: {err}"
    );
    assert_only_the_redirect_was_sent(&server).await;
}

#[tokio::test]
async fn an_ordinary_redirect_is_still_followed_under_readonly() {
    let server = MockServer::start().await;
    let target = format!("{}/plain", endpoint(OUTSIDE_HOST, &server));
    Mock::given(method("GET"))
        .and(path("/r"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", target.as_str()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/plain"))
        .respond_with(ResponseTemplate::new(200).set_body_string("plain ok"))
        .mount(&server)
        .await;
    let start = format!("{}/r", endpoint(OUTSIDE_HOST, &server));

    let vm = readonly_vm(&server, None);
    let body = eval(&vm, &format!(r#"return http.get("{start}").body"#))
        .await
        .unwrap();
    assert_eq!(body, "plain ok");
}

// ------------------------------------------------ a host that is not a name

const SIGV2_QUERY: &str =
    "AWSAccessKeyId=AKIA&SignatureVersion=2&SignatureMethod=HmacSHA256&Signature=ab";
const FOREIGN: &str = "aws-signed request to a host that is not an AWS endpoint";

/// An IP literal is not an AWS name, but it reaches an AWS endpoint just the
/// same. A SigV4-signed termination, with no `Host` header to give it away.
#[tokio::test]
async fn a_sigv4_signed_get_to_an_ip_literal_is_refused() {
    let server = MockServer::start().await;
    let url = format!("{}/?{TERMINATE_QUERY}", server.uri());
    let auth = format!("AWS4-HMAC-SHA256 Credential={SCOPE}, SignedHeaders=host, Signature=ab");
    let script = get_with_headers(&url, &format!("{{ authorization = \"{auth}\" }}"));
    refused_before_sending(&server, &script, FOREIGN).await;
}

#[tokio::test]
async fn sigv2_to_an_ip_literal_is_refused() {
    let server = MockServer::start().await;
    let url = format!("{}/?{TERMINATE_QUERY}&{SIGV2_QUERY}", server.uri());
    refused_before_sending(&server, &get_with_headers(&url, "{}"), FOREIGN).await;
}

#[tokio::test]
async fn sigv2_to_a_wildcard_dns_alias_is_refused() {
    let server = MockServer::start().await;
    let url = format!(
        "{}/?{TERMINATE_QUERY}&{SIGV2_QUERY}",
        endpoint(SSLIP_HOST, &server)
    );
    refused_before_sending(&server, &get_with_headers(&url, "{}"), FOREIGN).await;
}

/// The carve-out: an S3-compatible store (MinIO) on an address speaks SigV4
/// for `s3`, and its reads keep working.
#[tokio::test]
async fn an_s3_scoped_read_from_a_store_on_an_ip_still_runs() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/bucket/key"))
        .respond_with(ResponseTemplate::new(200).set_body_string("object"))
        .mount(&server)
        .await;
    let url = format!("{}/bucket/key", server.uri());
    let scope = "AKIAIOSFODNN7EXAMPLE/20260101/us-east-1/s3/aws4_request";
    let auth = format!("AWS4-HMAC-SHA256 Credential={scope}, SignedHeaders=host, Signature=ab");
    let script =
        format!(r#"return http.get("{url}", {{ headers = {{ authorization = "{auth}" }} }}).body"#);

    let vm = readonly_vm(&server, None);
    assert_eq!(eval(&vm, &script).await.unwrap(), "object");
}

/// The legacy SQS host is `queue`, not `sqs`. An `sqs` scope there is backed,
/// and `DeleteQueue` is not a read.
#[tokio::test]
async fn deleting_a_queue_on_the_legacy_sqs_host_is_refused() {
    let server = MockServer::start().await;
    let url = format!(
        "{}/?Action=DeleteQueue&QueueUrl=x",
        endpoint(QUEUE_HOST, &server)
    );
    let scope = "AKIAIOSFODNN7EXAMPLE/20260101/us-east-1/sqs/aws4_request";
    let auth = format!("AWS4-HMAC-SHA256 Credential={scope}, SignedHeaders=host, Signature=ab");
    let script = get_with_headers(&url, &format!("{{ authorization = \"{auth}\" }}"));
    refused_before_sending(&server, &script, "aws sqs Action=DeleteQueue is not a read").await;
}

/// A readable scope the AWS host does not serve says nothing about what the
/// request does there, so it is not left to its verb.
#[tokio::test]
async fn a_scope_the_aws_host_does_not_serve_is_refused() {
    let server = MockServer::start().await;
    let url = format!("{}/prod/items", endpoint(GATEWAY_HOST, &server));
    let auth = format!("AWS4-HMAC-SHA256 Credential={SCOPE}, SignedHeaders=host, Signature=ab");
    let script = get_with_headers(&url, &format!("{{ authorization = \"{auth}\" }}"));
    let why = "aws request is signed for a service this host does not serve";
    refused_before_sending(&server, &script, why).await;
}

/// `http.download` follows redirects too, and each hop is held to the policy.
#[tokio::test]
async fn a_download_redirected_off_the_policy_is_refused() {
    let server = MockServer::start().await;
    let start = mount_redirect_to_termination(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("out.bin");

    let rule = format!("    - hosts: [\"{OUTSIDE_HOST}\"]\n");
    let vm = vm_in_mode(&server, ExecMode::Unrestricted, Some(&policy(&[rule])));
    let script = format!(
        r#"http.download("{start}", "{dest}")
        return "reached""#,
        dest = dest.display()
    );
    let err = eval_err(&vm, &script).await;

    assert!(
        err.contains("is not allowed by any http rule"),
        "got: {err}"
    );
    assert_only_the_redirect_was_sent(&server).await;
    assert!(!dest.exists());
}

// ---------------------------------------- an s3 scope that names an operation

const S3_SCOPE: &str = "AKIAIOSFODNN7EXAMPLE/20260101/us-east-1/s3/aws4_request";

/// The S3-compatible carve-out covers object reads, not a Query-protocol
/// operation under an `s3` scope.
#[tokio::test]
async fn an_s3_signed_termination_to_an_ip_is_refused() {
    let server = MockServer::start().await;
    let url = format!("{}/?{TERMINATE_QUERY}", server.uri());
    let auth = format!("AWS4-HMAC-SHA256 Credential={S3_SCOPE}, SignedHeaders=host, Signature=ab");
    let script = get_with_headers(&url, &format!("{{ authorization = \"{auth}\" }}"));
    refused_before_sending(&server, &script, FOREIGN).await;
}

#[tokio::test]
async fn an_s3_presigned_termination_to_an_ip_is_refused() {
    let server = MockServer::start().await;
    let presign = "X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20260101%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Signature=ab";
    let url = format!("{}/?{TERMINATE_QUERY}&{presign}", server.uri());
    refused_before_sending(&server, &get_with_headers(&url, "{}"), FOREIGN).await;
}

/// A bucket on a legacy dash-region endpoint is still S3, and its signed
/// reads still run.
#[tokio::test]
async fn an_s3_read_from_a_legacy_endpoint_still_runs() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/key"))
        .respond_with(ResponseTemplate::new(200).set_body_string("object"))
        .mount(&server)
        .await;
    let url = format!("{}/key", endpoint(LEGACY_S3_HOST, &server));
    let auth = format!("AWS4-HMAC-SHA256 Credential={S3_SCOPE}, SignedHeaders=host, Signature=ab");
    let script =
        format!(r#"return http.get("{url}", {{ headers = {{ authorization = "{auth}" }} }}).body"#);

    let vm = readonly_vm(&server, None);
    assert_eq!(eval(&vm, &script).await.unwrap(), "object");
}
