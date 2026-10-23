use super::*;

fn probe(method: &str, url: &str) -> HttpCall {
    HttpCall {
        method: method.to_string(),
        url: Some(url.to_string()),
        headers: Vec::new(),
        headers_collide: false,
        body: None,
    }
}

/// The shape `sigv4.sign` produces: the scope in an `Authorization`
/// header, the service its fourth field.
fn signed(mut call: HttpCall, service: &str) -> HttpCall {
    let scope = format!("AKIA/20260101/us-east-1/{service}/aws4_request");
    let value = format!("{SIGV4_ALGORITHM} Credential={scope},SignedHeaders=host,Signature=d1d1");
    call.headers.push(("Authorization".to_string(), value));
    call
}

fn with_header(mut call: HttpCall, name: &str, value: &str) -> HttpCall {
    call.headers.push((name.to_string(), value.to_string()));
    call
}

fn parsed(call: &HttpCall) -> url::Url {
    url::Url::parse(call.url.as_deref().unwrap()).unwrap()
}

/// The service a signature on this call actually buys, host included.
fn service_of(call: &HttpCall) -> Option<String> {
    let url = parsed(call);
    let labels = aws_host_labels(&url).unwrap_or_default();
    match authentication(call, &url, false) {
        Auth::Scope(service) if host_serves(&labels, &service) => Some(service),
        _ => None,
    }
}

fn labels_of(host: &str) -> Vec<String> {
    let url = url::Url::parse(&format!("https://{host}/")).unwrap();
    aws_host_labels(&url).expect("an AWS host")
}

const EC2: &str = "https://ec2.us-east-1.amazonaws.com/";
const STS: &str = "https://sts.us-east-1.amazonaws.com/";
const ECR: &str = "https://api.ecr.us-east-1.amazonaws.com/";
/// An AWS-hosted endpoint whose content is the customer's, and which does
/// not verify SigV4 — the shape the host check exists for.
const GATEWAY: &str = "https://abc123.execute-api.us-east-1.amazonaws.com/prod/wipe";

// ----------------------------------------------------------------- hosts

#[test]
fn aws_hosts_match_on_label_boundaries() {
    let hosts = [
        "amazonaws.com",
        "ec2.us-east-1.amazonaws.com",
        "ec2.cn-north-1.amazonaws.com.cn",
        "api.aws",
        "ec2.api.aws",
        "EC2.US-EAST-1.AmazonAWS.com",
        "ec2.us-east-1.amazonaws.com.",
    ];
    for host in hosts {
        let url = url::Url::parse(&format!("https://{host}/")).unwrap();
        let labels = aws_host_labels(&url);
        assert!(labels.is_some(), "{host} should be an AWS host");
    }
}

#[test]
fn lookalike_hosts_are_not_aws() {
    let hosts = [
        "evilamazonaws.com",
        "amazonaws.com.evil.example",
        "notapi.aws.example.com",
        "amazonaws.com.cn.evil.example",
        "127.0.0.1",
        "[::1]",
    ];
    for host in hosts {
        let url = url::Url::parse(&format!("https://{host}/")).unwrap();
        let labels = aws_host_labels(&url);
        assert!(labels.is_none(), "{host} is not an AWS host");
    }
}

#[test]
fn a_signature_on_a_non_aws_host_is_a_write() {
    let forged = signed(probe("post", "https://evil.example.com/"), "sts");
    assert_eq!(classify(&forged), Some(Classification::Write));
}

// ------------------------------------------- the host owns the service

#[test]
fn a_service_claim_an_aws_host_does_not_back_is_a_write() {
    // An API Gateway stage is AWS-hosted, serves the customer's own code,
    // and never verifies SigV4, so an `sts` claim there buys no exemption.
    let mut forged = signed(probe("post", GATEWAY), "sts");
    forged.body = Some("Action=AssumeRole&Version=2011-06-15".to_string());
    assert_eq!(service_of(&forged), None);
    assert_eq!(classify(&forged), Some(Classification::Write));
    let want = "aws request is signed for a service this host does not serve";
    assert_eq!(refusal_reason(&forged).as_deref(), Some(want));
}

#[test]
fn a_bucket_named_for_a_service_is_not_that_service() {
    // `sts.s3.us-east-1.amazonaws.com` is a bucket called `sts`, and a
    // `POST` to it uploads an object. No exemption.
    let url = "https://sts.s3.us-east-1.amazonaws.com/";
    let mut forged = signed(probe("post", url), "sts");
    forged.body = Some("Action=AssumeRole".to_string());
    assert_eq!(service_of(&forged), None);
    assert_eq!(classify(&forged), Some(Classification::Write));
}

#[test]
fn an_unsigned_action_parameter_on_a_non_api_aws_host_is_left_alone() {
    // The case this must not break: an API Gateway route with an `action`
    // query parameter of its own. Nothing here is an AWS API call, so the
    // verb decides and this `GET` runs as it always did.
    let url = format!("{GATEWAY}?action=ping");
    assert_eq!(classify(&probe("get", &url)), None);

    // Two of them is not an AWS duplicate-`Action` refusal either.
    let twice = format!("{GATEWAY}?action=ping&action=pong");
    assert_eq!(classify(&probe("get", &twice)), None);
}

#[test]
fn a_signature_on_an_unrelated_aws_host_is_a_write_on_any_verb() {
    // Where the scope and the host disagree nothing says what AWS would do,
    // so not even a `GET` falls back to its verb.
    let get = signed(probe("get", GATEWAY), "sts");
    assert_eq!(classify(&get), Some(Classification::Write));
    let post = signed(probe("post", GATEWAY), "sts");
    assert_eq!(classify(&post), Some(Classification::Write));
}

#[test]
fn a_legacy_sqs_host_backs_an_sqs_scope() {
    assert!(host_serves(&labels_of("queue.amazonaws.com"), "sqs"));
    assert!(host_serves(
        &labels_of("us-east-1.queue.amazonaws.com"),
        "sqs"
    ));
    assert!(!host_serves(&labels_of("queue.amazonaws.com"), "ec2"));
    let url = "https://queue.amazonaws.com/?Action=DeleteQueue";
    let call = signed(probe("get", url), "sqs");
    assert_eq!(classify(&call), Some(Classification::Write));
    let want = "aws sqs Action=DeleteQueue is not a read";
    assert_eq!(refusal_reason(&call).as_deref(), Some(want));
}

#[test]
fn an_unbacked_scope_on_an_api_host_is_a_write_even_for_a_read() {
    let url = format!("{STS}?Action=GetCallerIdentity");
    let call = signed(probe("get", &url), "ec2");
    assert_eq!(classify(&call), Some(Classification::Write));
}

// ------------------------------------------------- hosts that are not names

const IP_TERMINATE: &str = "http://203.0.113.7/?Action=TerminateInstances&InstanceId.1=i-1";
const SIGV2: &str =
    "AWSAccessKeyId=AKIA&SignatureVersion=2&SignatureMethod=HmacSHA256&Signature=ab";
const FOREIGN: &str = "aws-signed request to a host that is not an AWS endpoint";

#[test]
fn a_sigv4_signed_get_to_an_ip_literal_is_a_write() {
    let call = signed(probe("get", IP_TERMINATE), "ec2");
    assert_eq!(classify(&call), Some(Classification::Write));
    assert_eq!(refusal_reason(&call).as_deref(), Some(FOREIGN));
}

#[test]
fn sigv2_to_an_ip_literal_or_an_alias_is_a_write() {
    for url in [
        format!("{IP_TERMINATE}&{SIGV2}"),
        format!("http://203-0-113-7.sslip.io/?Action=TerminateInstances&{SIGV2}"),
    ] {
        assert_eq!(
            classify(&probe("get", &url)),
            Some(Classification::Write),
            "{url}"
        );
    }
}

#[test]
fn any_aws_signing_material_off_aws_is_a_write() {
    let presign = format!("{IP_TERMINATE}&X-Amz-Signature=ab");
    assert_eq!(
        classify(&probe("get", &presign)),
        Some(Classification::Write)
    );
    let algorithm = format!("{IP_TERMINATE}&X-Amz-Algorithm=AWS4-HMAC-SHA256");
    assert_eq!(
        classify(&probe("get", &algorithm)),
        Some(Classification::Write)
    );
    let lower = with_header(
        probe("get", IP_TERMINATE),
        "authorization",
        "aws4-hmac-sha256 x",
    );
    assert_eq!(classify(&lower), Some(Classification::Write));
}

#[test]
fn an_s3_compatible_store_keeps_its_reads() {
    let object = "http://203.0.113.7:9000/bucket/key";
    assert_eq!(
        classify(&signed(probe("get", object), "s3")),
        Some(Classification::Read)
    );
    assert_eq!(
        classify(&signed(probe("put", object), "s3")),
        Some(Classification::Write)
    );
    assert_eq!(
        classify(&signed(probe("get", object), "ec2")),
        Some(Classification::Write)
    );
}

#[test]
fn a_host_off_aws_with_no_signing_material_is_left_to_the_verb() {
    assert_eq!(classify(&probe("get", IP_TERMINATE)), None);
    let half_sigv2 = format!("{IP_TERMINATE}&Signature=mine");
    assert_eq!(classify(&probe("get", &half_sigv2)), None);
    let bearer = with_header(probe("get", IP_TERMINATE), "authorization", "Bearer x");
    assert_eq!(classify(&bearer), None);
}

#[test]
fn a_host_serves_the_service_its_tail_names() {
    let owned = [
        ("sts.us-east-1.amazonaws.com", "sts"),
        ("sts.amazonaws.com", "sts"),
        ("sts-fips.us-east-1.amazonaws.com", "sts"),
        ("sts.cn-north-1.amazonaws.com.cn", "sts"),
        ("vpce-0a1b.sts.us-east-1.vpce.amazonaws.com", "sts"),
        ("api.ecr.us-east-1.amazonaws.com", "ecr"),
        ("ec2.us-east-1.api.aws", "ec2"),
        ("my-bucket.s3.us-east-1.amazonaws.com", "s3"),
        ("s3.dualstack.us-east-1.amazonaws.com", "s3"),
        ("s3.s3.us-east-1.amazonaws.com", "s3"),
    ];
    for (host, service) in owned {
        let labels = labels_of(host);
        assert!(host_serves(&labels, service), "{host} serves {service}");
    }
}

#[test]
fn a_host_does_not_serve_a_service_it_only_mentions() {
    let disowned = [
        ("abc123.execute-api.us-east-1.amazonaws.com", "sts"),
        ("sts.s3.us-east-1.amazonaws.com", "sts"),
        ("sts.s3-us-east-1.amazonaws.com", "sts"),
        ("sts.us-east-1.s3.amazonaws.com", "sts"),
        ("my-lb-1234.us-east-1.elb.amazonaws.com", "sts"),
        ("ec2.us-east-1.amazonaws.com", "sts"),
        ("api.ecr.us-east-1.amazonaws.com", "ec2"),
    ];
    for (host, service) in disowned {
        let labels = labels_of(host);
        assert!(!host_serves(&labels, service), "{host} is not {service}");
    }
}

#[test]
fn a_region_is_an_area_and_a_number_not_any_hyphenated_word() {
    for region in ["us-east-1", "eu-west-3", "cn-north-1", "us-gov-west-1"] {
        assert!(is_region(region), "{region} is a region");
    }
    for other in ["s3-us-east-1", "execute-api", "dualstack", "us", "elb"] {
        assert!(!is_region(other), "{other} is not a region");
    }
}

// ------------------------------------------------------ credential scope

#[test]
fn the_service_comes_from_the_credential_scope() {
    let call = signed(probe("get", EC2), "ec2");
    assert_eq!(service_of(&call).as_deref(), Some("ec2"));
}

#[test]
fn a_presigned_request_carries_its_scope_in_the_query() {
    let scope = "AKIA%2F20260101%2Fus-east-1%2Fsts%2Faws4_request";
    let url = format!("{STS}?Action=GetCallerIdentity&X-Amz-Credential={scope}");
    let call = probe("get", &url);
    assert_eq!(service_of(&call).as_deref(), Some("sts"));
    assert_eq!(classify(&call), Some(Classification::Read));
}

#[test]
fn a_malformed_scope_counts_as_unsigned() {
    let bad = [
        "AKIA/20260101/us-east-1/ec2",
        "AKIA/20260101/us-east-1/ec2/aws4_request/extra",
        "AKIA/20260101/us-east-1//aws4_request",
        "AKIA/20260101/us-east-1/ec2/not_aws4",
        "nonsense",
    ];
    for credential in bad {
        assert_eq!(service_of_scope(credential), None, "{credential}");
    }
}

#[test]
fn a_signature_that_is_not_sigv4_counts_as_unsigned() {
    let call = with_header(probe("post", STS), "authorization", "Bearer t");
    assert_eq!(service_of(&call), None);
}

// -------------------------------------------------------------- actions

#[test]
fn the_action_can_come_from_the_query() {
    let call = signed(
        probe("get", &format!("{EC2}?Action=DescribeInstances")),
        "ec2",
    );
    assert_eq!(classify(&call), Some(Classification::Read));
}

#[test]
fn the_action_can_come_from_the_form_body() {
    let mut call = signed(probe("post", STS), "sts");
    call.body = Some("Action=AssumeRole&Version=2011-06-15".to_string());
    assert_eq!(classify(&call), Some(Classification::Read));
}

#[test]
fn the_action_can_come_from_the_target_header() {
    let target = "AmazonEC2ContainerRegistry_V20150921.GetAuthorizationToken";
    let mut call = with_header(signed(probe("post", ECR), "ecr"), "x-amz-target", target);
    call.body = Some("{}".to_string());
    assert_eq!(classify(&call), Some(Classification::Read));
}

#[test]
fn a_duplicate_action_is_refused() {
    let url = format!("{EC2}?Action=DescribeInstances&Action=TerminateInstances");
    let call = signed(probe("get", &url), "ec2");
    assert_eq!(classify(&call), Some(Classification::Write));
    let reason = refusal_reason(&call);
    assert_eq!(
        reason.as_deref(),
        Some("aws request carries more than one Action")
    );
}

#[test]
fn an_action_split_between_query_and_body_is_still_a_duplicate() {
    let url = format!("{EC2}?Action=DescribeInstances");
    let mut call = signed(probe("post", &url), "ec2");
    call.body = Some("Action=DescribeInstances".to_string());
    assert_eq!(classify(&call), Some(Classification::Write));
}

#[test]
fn a_target_disagreeing_with_the_action_is_refused() {
    let target = "AmazonEC2ContainerRegistry_V20150921.BatchDeleteImage";
    let url = format!("{ECR}?Action=GetAuthorizationToken");
    let call = with_header(signed(probe("post", &url), "ecr"), "X-Amz-Target", target);
    assert_eq!(classify(&call), Some(Classification::Write));
    let reason = refusal_reason(&call);
    assert_eq!(
        reason.as_deref(),
        Some("aws request's X-Amz-Target and Action disagree")
    );
}

// ------------------------------------------------------ decision table

#[test]
fn the_allowlisted_reads_are_reads_on_any_verb() {
    for (service, action) in READ_ACTIONS {
        let url = format!("https://{service}.us-east-1.amazonaws.com/?Action={action}");
        for method in ["get", "post"] {
            let call = signed(probe(method, &url), service);
            let what = format!("{method} {service} {action}");
            assert_eq!(classify(&call), Some(Classification::Read), "{what}");
        }
    }
}

#[test]
fn a_mutating_action_is_refused_whatever_the_verb() {
    let url = format!("{EC2}?Action=TerminateInstances");
    for method in ["get", "post", "put", "delete"] {
        let call = signed(probe(method, &url), "ec2");
        assert_eq!(classify(&call), Some(Classification::Write), "{method}");
        let reason = refusal_reason(&call);
        let want = "aws ec2 Action=TerminateInstances is not a read";
        assert_eq!(reason.as_deref(), Some(want), "{method}");
    }
}

#[test]
fn credential_minting_is_not_a_read_however_it_is_named() {
    for action in ["GetSessionToken", "GetFederationToken"] {
        let call = signed(probe("post", &format!("{STS}?Action={action}")), "sts");
        assert_eq!(classify(&call), Some(Classification::Write), "{action}");
    }
}

#[test]
fn the_web_identity_exchange_is_a_read_unsigned_on_an_sts_host() {
    let body = format!("Action={WEB_IDENTITY_ACTION}&Version=2011-06-15");

    let mut exchange = probe("post", STS);
    exchange.body = Some(body.clone());
    assert_eq!(classify(&exchange), Some(Classification::Read));

    let mut fips = probe("post", "https://sts-fips.us-east-1.amazonaws.com/");
    fips.body = Some(body.clone());
    assert_eq!(classify(&fips), Some(Classification::Read));

    // The same unsigned body aimed at another service's host is not it:
    // EC2 has no such read, so it is a write.
    let mut elsewhere = probe("post", EC2);
    elsewhere.body = Some(body);
    assert_eq!(classify(&elsewhere), Some(Classification::Write));
}

#[test]
fn an_unsigned_action_other_than_the_exchange_is_left_to_the_verb() {
    // `AssumeRole` proper is signed with base credentials, so unsigned it
    // is not the exchange. Nothing to exempt, and nothing to refuse on
    // beyond the verb, which still makes this `POST` a write.
    let mut call = probe("post", STS);
    call.body = Some("Action=AssumeRole&Version=2011-06-15".to_string());
    assert_eq!(classify(&call), None);
}

#[test]
fn a_duplicate_action_unsigned_cannot_pass_as_the_exchange() {
    let mut call = probe("post", STS);
    let body = format!("Action={WEB_IDENTITY_ACTION}&Action=GetSessionToken");
    call.body = Some(body);
    assert_eq!(classify(&call), Some(Classification::Write));
}

#[test]
fn an_unsigned_request_with_no_action_falls_back_to_the_verb() {
    let object = "https://my-bucket.s3.amazonaws.com/key";
    assert_eq!(classify(&probe("get", object)), None);
    assert_eq!(classify(&probe("post", object)), None);
}

#[test]
fn a_signed_rest_read_is_a_read_and_a_signed_rest_write_is_not() {
    let read = signed(probe("get", "https://s3.us-east-1.amazonaws.com/b"), "s3");
    assert_eq!(classify(&read), Some(Classification::Read));

    let write = signed(probe("put", "https://s3.us-east-1.amazonaws.com/b/k"), "s3");
    assert_eq!(classify(&write), Some(Classification::Write));
    let reason = refusal_reason(&write);
    assert_eq!(reason.as_deref(), Some("aws s3 PUT is not a read"));
}

#[test]
fn a_signed_request_to_an_unknown_service_is_a_write() {
    let call = signed(
        probe("get", "https://logs.us-east-1.amazonaws.com/"),
        "logs",
    );
    assert_eq!(classify(&call), Some(Classification::Write));
}

#[test]
fn a_hostile_action_name_stays_out_of_the_message() {
    let url = format!("{EC2}?Action=Terminate%20%3Cscript%3E");
    let call = signed(probe("post", &url), "ec2");
    let reason = refusal_reason(&call);
    assert_eq!(reason.as_deref(), Some("aws ec2 action is not a read"));
}

// ------------------------------------------------------ what is sent

const TERMINATE: &str = "https://ec2.us-east-1.amazonaws.com/?Action=TerminateInstances";
const REAL_SIGNATURE: &str = "AWS4-HMAC-SHA256 Credential=AKIA/20260101/us-east-1/ec2/aws4_request, SignedHeaders=host;x-amz-date, Signature=d1d1";

/// `http.get(url, {headers = ...})` read the way the gates read it, with
/// the headers set in the order given.
fn from_lua(url: &str, headers: &[(&str, &str)]) -> HttpCall {
    let lua = mlua::Lua::new();
    let table = lua.create_table().unwrap();
    for (name, value) in headers {
        table.set(*name, *value).unwrap();
    }
    let opts = lua.create_table().unwrap();
    opts.set("headers", table).unwrap();
    let url = lua.create_string(url).unwrap();
    let args = mlua::MultiValue::from_vec(vec![mlua::Value::String(url), mlua::Value::Table(opts)]);
    super::super::http_call_from_args("get", &args, 0).unwrap()
}

/// The transport sends one value per lowercased name, so a second
/// `Authorization` spelled differently would let the classifier read one
/// value while AWS receives the other.
#[test]
fn a_header_named_twice_in_different_case_is_refused_in_either_order() {
    let bearer = ("Authorization", "Bearer x");
    let real = ("authorization", REAL_SIGNATURE);
    let shouted = ("AUTHORIZATION", REAL_SIGNATURE);
    for headers in [
        [bearer, real],
        [real, bearer],
        [bearer, shouted],
        [shouted, bearer],
    ] {
        let call = from_lua(TERMINATE, &headers);
        assert!(call.headers_collide, "{headers:?}");
        assert_eq!(classify(&call), Some(Classification::Write), "{headers:?}");
        let reason = refusal_reason(&call);
        assert_eq!(reason.as_deref(), Some("aws request names a header twice"));
    }
}

/// Whichever value the transport keeps, the refusal does not depend on it:
/// a surviving bearer token leaves no signature to classify by, and
/// without the collision check the verb would admit this `GET`.
#[test]
fn a_collision_is_refused_whichever_value_is_sent() {
    for sent in ["Bearer x", REAL_SIGNATURE] {
        let mut call = with_header(probe("get", TERMINATE), "authorization", sent);
        call.headers_collide = true;
        assert_eq!(classify(&call), Some(Classification::Write), "{sent}");
    }
}

#[test]
fn the_classifier_reads_the_one_value_the_transport_sends() {
    let call = from_lua(
        TERMINATE,
        &[
            ("Authorization", "Bearer x"),
            ("authorization", REAL_SIGNATURE),
        ],
    );
    let authorization: Vec<_> = call
        .headers
        .iter()
        .filter(|(name, _)| name == "authorization")
        .collect();
    assert_eq!(authorization.len(), 1, "{:?}", call.headers);
}

#[test]
fn distinct_header_names_do_not_collide() {
    let url = format!("{EC2}?Action=DescribeInstances");
    let headers = [
        ("Authorization", REAL_SIGNATURE),
        ("X-Amz-Date", "20260101T000000Z"),
    ];
    let call = from_lua(&url, &headers);
    assert!(!call.headers_collide);
    assert_eq!(classify(&call), Some(Classification::Read));
}

#[test]
fn a_collision_off_aws_is_left_to_the_verb() {
    let headers = [("Authorization", "Bearer x"), ("authorization", "Bearer y")];
    let call = from_lua("https://example.com/", &headers);
    assert!(call.headers_collide);
    assert_eq!(classify(&call), None);
}

#[test]
fn a_duplicate_credential_scope_is_refused_in_either_order() {
    let ec2 = "X-Amz-Credential=AKIA%2F20260101%2Fus-east-1%2Fec2%2Faws4_request";
    let sts = "x-amz-credential=AKIA%2F20260101%2Fus-east-1%2Fsts%2Faws4_request";
    for (first, second) in [(ec2, sts), (sts, ec2), (ec2, ec2)] {
        let url = format!("{STS}?Action=GetCallerIdentity&{first}&{second}");
        let call = probe("get", &url);
        assert_eq!(classify(&call), Some(Classification::Write), "{url}");
        let reason = refusal_reason(&call);
        let want = "aws request carries more than one X-Amz-Credential";
        assert_eq!(reason.as_deref(), Some(want), "{url}");
    }
}

#[test]
fn an_action_repeated_in_different_case_is_a_duplicate() {
    for query in [
        "Action=DescribeInstances&action=TerminateInstances",
        "action=TerminateInstances&Action=DescribeInstances",
        "ACTION=DescribeInstances&Action=DescribeInstances",
    ] {
        let call = signed(probe("get", &format!("{EC2}?{query}")), "ec2");
        assert_eq!(classify(&call), Some(Classification::Write), "{query}");
        let reason = refusal_reason(&call);
        assert_eq!(
            reason.as_deref(),
            Some("aws request carries more than one Action")
        );
    }
}

// ------------------------------------------ decided by host, not parse

const UNREADABLE: &str = "aws request carries a signature that cannot be read";

fn authorized(url: &str, authorization: &str) -> HttpCall {
    with_header(probe("get", url), "authorization", authorization)
}

#[test]
fn the_algorithm_is_matched_in_any_case() {
    let lower = REAL_SIGNATURE.replacen("AWS4-HMAC-SHA256", "aws4-hmac-sha256", 1);
    let call = authorized(TERMINATE, &lower);
    assert_eq!(service_of(&call).as_deref(), Some("ec2"));
    assert_eq!(classify(&call), Some(Classification::Write));
    let want = "aws ec2 Action=TerminateInstances is not a read";
    assert_eq!(refusal_reason(&call).as_deref(), Some(want));
}

#[test]
fn a_decoy_component_makes_the_signature_unreadable() {
    let decoy = REAL_SIGNATURE.replacen("Credential=", "Foo=Credential=x, Credential=", 1);
    let call = authorized(TERMINATE, &decoy);
    assert_eq!(classify(&call), Some(Classification::Write));
    assert_eq!(refusal_reason(&call).as_deref(), Some(UNREADABLE));
}

#[test]
fn a_sigv4_header_must_carry_each_component_once() {
    let scope = "Credential=AKIA/20260101/us-east-1/ec2/aws4_request";
    let broken = [
        format!("AWS4-HMAC-SHA256 {scope}, Signature=d1"),
        format!("AWS4-HMAC-SHA256 {scope}, SignedHeaders=host, Signature=d1, Signature=d2"),
        format!("AWS4-HMAC-SHA256 {scope}, {scope}, SignedHeaders=host, Signature=d1"),
        format!("AWS4-HMAC-SHA256 {scope}, SignedHeaders=, Signature=d1"),
        "AWS4-HMAC-SHA256".to_string(),
        "AWS AKIA:c2lnbmF0dXJl".to_string(),
        "AWS4-ECDSA-P256-SHA256 Credential=x".to_string(),
    ];
    let url = format!("{EC2}?Action=DescribeInstances");
    for value in broken {
        let call = authorized(&url, &value);
        assert_eq!(classify(&call), Some(Classification::Write), "{value}");
        assert_eq!(
            refusal_reason(&call).as_deref(),
            Some(UNREADABLE),
            "{value}"
        );
    }
}

#[test]
fn sigv2_query_authentication_is_unreadable_on_an_api_host() {
    let sigv2 = "AWSAccessKeyId=AKIA&SignatureVersion=2&SignatureMethod=HmacSHA256&Signature=abcd";
    let call = probe("get", &format!("{TERMINATE}&{sigv2}"));
    assert_eq!(classify(&call), Some(Classification::Write));
    assert_eq!(refusal_reason(&call).as_deref(), Some(UNREADABLE));

    // A read is no exception: SigV2 is not something this reads.
    let read = probe(
        "get",
        &format!("{EC2}?Action=DescribeInstances&Signature=abcd"),
    );
    assert_eq!(classify(&read), Some(Classification::Write));
}

#[test]
fn sigv2_names_on_a_customer_host_are_the_customer_s_own() {
    let url = format!("{GATEWAY}?Signature=abcd&AWSAccessKeyId=mine");
    assert_eq!(classify(&probe("get", &url)), None);
}

#[test]
fn a_malformed_presign_is_unreadable_on_any_aws_host() {
    let credential = "X-Amz-Credential=AKIA%2F20260101%2Fus-east-1%2Fec2%2Faws4_request";
    let broken = [
        format!("{TERMINATE}&X-Amz-Signature=abcd"),
        format!("{TERMINATE}&X-Amz-Credential=nonsense"),
        format!("{TERMINATE}&X-Amz-Algorithm=AWS4-HMAC-SHA1&{credential}"),
        format!("{GATEWAY}?X-Amz-Signature=abcd"),
    ];
    for url in broken {
        let call = probe("get", &url);
        assert_eq!(classify(&call), Some(Classification::Write), "{url}");
    }
}

#[test]
fn a_header_signature_and_a_presign_together_are_unreadable() {
    let presign = "X-Amz-Credential=AKIA%2F20260101%2Fus-east-1%2Fec2%2Faws4_request";
    let url = format!("{EC2}?Action=DescribeInstances&{presign}");
    let call = authorized(&url, REAL_SIGNATURE);
    assert_eq!(refusal_reason(&call).as_deref(), Some(UNREADABLE));
}

#[test]
fn an_unsigned_mutation_on_an_api_host_is_decided_on_its_action() {
    assert_eq!(
        classify(&probe("get", TERMINATE)),
        Some(Classification::Write)
    );
    let iam = "https://iam.amazonaws.com/?Action=DeleteUser&UserName=x";
    assert_eq!(classify(&probe("get", iam)), Some(Classification::Write));
    // A bearer token is not AWS authentication; the action still decides.
    let bearer = authorized(TERMINATE, "Bearer x");
    assert_eq!(classify(&bearer), Some(Classification::Write));
}

#[test]
fn an_unsigned_read_on_an_api_host_is_left_to_the_verb() {
    let url = format!("{EC2}?Action=DescribeInstances");
    assert_eq!(classify(&probe("get", &url)), None);
    assert_eq!(classify(&probe("get", EC2)), None);
}

#[test]
fn a_public_object_get_stays_with_the_verb() {
    let object = "https://my-bucket.s3.us-east-1.amazonaws.com/key?Action=whatever";
    assert_eq!(classify(&probe("get", object)), None);
}

#[test]
fn the_sovereign_and_iso_partitions_are_aws_hosts() {
    for host in [
        "ec2.eusc-de-east-1.amazonaws.eu",
        "ec2.us-iso-east-1.c2s.ic.gov",
        "ec2.us-isob-east-1.sc2s.sgov.gov",
    ] {
        let url = format!("https://{host}/?Action=TerminateInstances");
        assert_eq!(
            classify(&probe("get", &url)),
            Some(Classification::Write),
            "{host}"
        );
    }
}

#[test]
fn a_read_has_no_refusal_reason() {
    let url = format!("{EC2}?Action=DescribeInstances");
    assert_eq!(refusal_reason(&signed(probe("get", &url), "ec2")), None);
}

// ------------------------------------------------- an s3 scope that names an operation

const S3_OPERATION: &str = "aws s3-signed request names an operation";

#[test]
fn an_s3_scope_naming_an_action_off_aws_is_refused() {
    for url in [
        IP_TERMINATE,
        "http://203.0.113.7/?action=TerminateInstances",
    ] {
        let call = signed(probe("get", url), "s3");
        assert_eq!(classify(&call), Some(Classification::Write), "{url}");
        assert_eq!(refusal_reason(&call).as_deref(), Some(FOREIGN), "{url}");
    }
    let target = with_header(
        signed(probe("get", "http://203.0.113.7/bucket"), "s3"),
        "x-amz-target",
        "AmazonEC2.TerminateInstances",
    );
    assert_eq!(classify(&target), Some(Classification::Write));
}

#[test]
fn an_s3_presign_naming_an_action_off_aws_is_refused() {
    let presign =
        "X-Amz-Credential=AKIA%2F20260101%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Signature=ab";
    let call = probe("get", &format!("{IP_TERMINATE}&{presign}"));
    assert_eq!(classify(&call), Some(Classification::Write));

    let object = format!("http://203.0.113.7:9000/bucket/key?{presign}");
    assert_eq!(classify(&probe("get", &object)), Some(Classification::Read));
}

// ------------------------------------------------------ the AWS scheme exactly

#[test]
fn only_aws_s_own_schemes_are_signing_material() {
    let lookalike = with_header(
        probe("get", IP_TERMINATE),
        "authorization",
        "AWSomeToken abc",
    );
    assert_eq!(classify(&lookalike), None);
    let no_dash = with_header(probe("get", IP_TERMINATE), "authorization", "AWS4HMAC x");
    assert_eq!(classify(&no_dash), None);
    for scheme in ["AWS AKIA:c2ln", "aws AKIA:c2ln", "AWS4-ECDSA-P256-SHA256 x"] {
        let call = with_header(probe("get", IP_TERMINATE), "authorization", scheme);
        assert_eq!(classify(&call), Some(Classification::Write), "{scheme}");
    }
}

#[test]
fn a_lookalike_scheme_on_an_api_host_leaves_the_action_to_decide() {
    let call = with_header(probe("get", TERMINATE), "authorization", "AWSomeToken abc");
    assert_eq!(classify(&call), Some(Classification::Write));
    assert_ne!(refusal_reason(&call).as_deref(), Some(UNREADABLE));
}

// ------------------------------------------- S3 hosts without the s3 label

#[test]
fn s3_reads_work_on_access_point_and_legacy_hosts() {
    let hosts = [
        "https://my-bucket.s3-us-west-2.amazonaws.com/key",
        "https://s3-us-west-2.amazonaws.com/my-bucket/key",
        "https://s3-external-1.amazonaws.com/my-bucket/key",
        "https://myap-123456789012.s3-accesspoint.us-east-1.amazonaws.com/key",
        "https://myap-123456789012.s3-accesspoint.dualstack.us-east-1.amazonaws.com/key",
    ];
    for url in hosts {
        let read = signed(probe("get", url), "s3");
        assert_eq!(classify(&read), Some(Classification::Read), "{url}");
        let write = signed(probe("put", url), "s3");
        assert_eq!(classify(&write), Some(Classification::Write), "{url}");
        let other = signed(probe("get", url), "ec2");
        assert_eq!(classify(&other), Some(Classification::Write), "{url}");
    }
}

#[test]
fn a_legacy_s3_host_still_refuses_a_named_operation() {
    let url = "https://s3-us-west-2.amazonaws.com/?Action=TerminateInstances";
    let call = signed(probe("get", url), "s3");
    assert_eq!(classify(&call), Some(Classification::Write));
    assert_eq!(refusal_reason(&call).as_deref(), Some(S3_OPERATION));
}

#[test]
fn a_dash_label_that_is_not_a_region_is_not_s3() {
    for url in [
        "https://bucket.s3-website-us-east-1.amazonaws.com/key",
        "https://s3-us-west-2.evil.amazonaws.com/key",
    ] {
        let call = signed(probe("get", url), "s3");
        assert_eq!(classify(&call), Some(Classification::Write), "{url}");
    }
}
