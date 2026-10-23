//! A read-only or approval VM whose HTTP client sends the real AWS hostnames
//! (and a few others) to a wiremock server, for the `assay.aws` gate tests.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use assay::lua::policy::Policy;
use assay::lua::{ExecMode, VmOptions, create_vm_with_policy};
use wiremock::MockServer;

pub const STS_HOST: &str = "sts.us-east-1.amazonaws.com";
pub const EC2_HOST: &str = "ec2.us-east-1.amazonaws.com";
pub const ECR_HOST: &str = "api.ecr.us-east-1.amazonaws.com";
/// An AWS host that is not an AWS API: an API Gateway stage serves the
/// customer's own code, never verifies SigV4, and has query parameters of its
/// own — so it is both what the host check protects and what it must not break.
pub const GATEWAY_HOST: &str = "abc123.execute-api.us-east-1.amazonaws.com";
/// Ends in `amazonaws.com` without a label boundary before it, which is the
/// case a suffix match would wave through.
pub const LOOKALIKE_HOST: &str = "evilamazonaws.com";
pub const OUTSIDE_HOST: &str = "not-aws.example.com";
/// The legacy SQS endpoint, whose label is `queue` rather than `sqs`.
pub const QUEUE_HOST: &str = "queue.amazonaws.com";
/// A wildcard-DNS name for an address: an AWS endpoint under a name that is
/// not AWS's.
pub const SSLIP_HOST: &str = "127-0-0-1.sslip.io";
/// A bucket on a legacy dash-region S3 endpoint, whose label is not `s3`.
pub const LEGACY_S3_HOST: &str = "my-bucket.s3-us-west-2.amazonaws.com";

pub const RESOLVED_HOSTS: &[&str] = &[
    STS_HOST,
    EC2_HOST,
    ECR_HOST,
    GATEWAY_HOST,
    LOOKALIKE_HOST,
    OUTSIDE_HOST,
    QUEUE_HOST,
    SSLIP_HOST,
    LEGACY_S3_HOST,
];

pub const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
pub const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

/// A client that sends every hostname this suite uses to the mock. A resolve
/// override supplies the address only — the port has to travel in the URL —
/// so the endpoints below carry the mock's port and the hostname stays real.
pub fn client(addr: SocketAddr) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .redirect(assay::lua::builtins::http::redirect_policy());
    for host in RESOLVED_HOSTS {
        builder = builder.resolve(host, addr);
    }
    builder.build().unwrap()
}

pub fn endpoint(host: &str, server: &MockServer) -> String {
    format!("http://{host}:{}", server.address().port())
}

/// What a signer puts in the `host` header for that endpoint. The mock's port
/// is part of the authority; the classifier reads the hostname and ignores it.
pub fn signed_host(host: &str, server: &MockServer) -> String {
    format!("{host}:{}", server.address().port())
}

pub fn vm_in_mode(server: &MockServer, mode: ExecMode, policy_yaml: Option<&str>) -> mlua::Lua {
    let parsed = policy_yaml.map(|yaml| Arc::new(Policy::parse(yaml).expect("policy parses")));
    create_vm_with_policy(
        client(server.address().to_owned()),
        VmOptions {
            mode,
            ..Default::default()
        },
        parsed,
    )
    .unwrap()
}

pub fn readonly_vm(server: &MockServer, policy_yaml: Option<&str>) -> mlua::Lua {
    vm_in_mode(server, ExecMode::ReadOnly, policy_yaml)
}

pub async fn eval(vm: &mlua::Lua, script: &str) -> mlua::Result<String> {
    vm.load(script).eval_async::<String>().await
}

/// The refusal a script ran into, as text.
pub async fn eval_err(vm: &mlua::Lua, script: &str) -> String {
    eval(vm, script).await.unwrap_err().to_string()
}

pub fn policy(rules: &[String]) -> String {
    format!("version: 1\nhttp:\n  rules:\n{}", rules.concat())
}
