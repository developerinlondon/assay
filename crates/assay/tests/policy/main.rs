use std::sync::Arc;

use assay::lua::policy::Policy;
use assay::lua::{ExecMode, VmOptions, create_vm_with_policy};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap()
}

fn vm(policy_yaml: &str, mode: ExecMode) -> mlua::Lua {
    let policy = Arc::new(Policy::parse(policy_yaml).expect("policy parses"));
    create_vm_with_policy(
        client(),
        VmOptions {
            mode,
            ..Default::default()
        },
        Some(policy),
    )
    .unwrap()
}

fn unpoliced() -> mlua::Lua {
    create_vm_with_policy(client(), VmOptions::default(), None).unwrap()
}

async fn eval(vm: &mlua::Lua, script: &str) -> mlua::Result<String> {
    vm.load(script).eval_async::<String>().await
}

// ---------------------------------------------------------------- modules

#[tokio::test]
async fn require_outside_the_allowlist_is_refused() {
    let vm = vm(
        "version: 1\nmodules:\n  allow: [assay.json]\n",
        ExecMode::Unrestricted,
    );
    let err = eval(&vm, r#"require("assay.openstack") return "loaded""#)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("module 'assay.openstack' is not in the allowed set"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn require_inside_the_allowlist_still_loads() {
    let vm = vm(
        "version: 1\nmodules:\n  allow: [assay.openstack]\n",
        ExecMode::Unrestricted,
    );
    let out = eval(&vm, r#"require("assay.openstack") return "loaded""#)
        .await
        .unwrap();
    assert_eq!(out, "loaded");
}

#[tokio::test]
async fn without_a_modules_section_every_require_is_allowed() {
    let vm = vm("version: 1\n", ExecMode::Unrestricted);
    let out = eval(&vm, r#"require("assay.openstack") return "loaded""#)
        .await
        .unwrap();
    assert_eq!(out, "loaded");
}

// -------------------------------------------------------------------- env

#[tokio::test]
async fn env_get_hides_keys_outside_the_allowlist() {
    unsafe {
        std::env::set_var("ASSAY_POLICY_TEST_SECRET", "hunter2");
        std::env::set_var("ASSAY_POLICY_TEST_PUBLIC", "fine");
    }
    let vm = vm(
        "version: 1\nenv:\n  allow: [ASSAY_POLICY_TEST_PUBLIC]\n",
        ExecMode::Unrestricted,
    );
    let out = eval(
        &vm,
        r#"return tostring(env.get("ASSAY_POLICY_TEST_SECRET")) .. "/" ..
           tostring(env.get("ASSAY_POLICY_TEST_PUBLIC"))"#,
    )
    .await
    .unwrap();
    assert_eq!(out, "nil/fine");
}

#[tokio::test]
async fn env_list_omits_keys_outside_the_allowlist() {
    unsafe {
        std::env::set_var("ASSAY_POLICY_TEST_SECRET", "hunter2");
    }
    let vm = vm("version: 1\nenv:\n  allow: []\n", ExecMode::Unrestricted);
    let out = eval(
        &vm,
        r#"local n = 0 for _ in ipairs(env.list()) do n = n + 1 end return tostring(n)"#,
    )
    .await
    .unwrap();
    assert_eq!(out, "0");
}

#[tokio::test]
async fn an_unpoliced_vm_reads_the_environment_as_before() {
    unsafe {
        std::env::set_var("ASSAY_POLICY_TEST_PUBLIC", "fine");
    }
    let vm = unpoliced();
    let out = eval(&vm, r#"return env.get("ASSAY_POLICY_TEST_PUBLIC")"#)
        .await
        .unwrap();
    assert_eq!(out, "fine");
}

// ------------------------------------------------------------------- http

#[tokio::test]
async fn a_host_outside_the_rules_is_refused_before_the_request() {
    let policy = "version: 1\nhttp:\n  rules:\n    - hosts: [\"allowed.example.com\"]\n";
    let vm = vm(policy, ExecMode::Unrestricted);
    let err = eval(
        &vm,
        r#"local r = http.get("http://127.0.0.1:1/x") return r.body"#,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("is not allowed by any http rule"),
        "expected a policy refusal, got: {err}"
    );
}

#[tokio::test]
async fn a_matching_rule_lets_the_request_through() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v3/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let policy = format!(
        "version: 1\nhttp:\n  rules:\n    - hosts: [\"{}\"]\n      methods: [GET]\n      paths: [\"/v3/*\"]\n",
        server.address().ip()
    );
    let vm = vm(&policy, ExecMode::Unrestricted);
    let out = eval(
        &vm,
        &format!(
            r#"local r = http.get("{}/v3/projects") return r.body"#,
            server.uri()
        ),
    )
    .await
    .unwrap();
    assert_eq!(out, "ok");
}

#[tokio::test]
async fn a_path_outside_the_rule_is_refused_on_an_allowed_host() {
    let server = MockServer::start().await;
    let policy = format!(
        "version: 1\nhttp:\n  rules:\n    - hosts: [\"{}\"]\n      methods: [GET]\n      paths: [\"/v3/*\"]\n",
        server.address().ip()
    );
    let vm = vm(&policy, ExecMode::Unrestricted);
    let err = eval(
        &vm,
        &format!(
            r#"local r = http.get("{}/admin/keys") return r.body"#,
            server.uri()
        ),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("is not allowed by any http rule"),
        "got: {err}"
    );
}

// --------------------------------------------------- semantic read (AS-3)

#[tokio::test]
async fn a_declared_read_post_proceeds_under_readonly() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v3/auth/tokens"))
        .respond_with(ResponseTemplate::new(201).set_body_string("issued"))
        .mount(&server)
        .await;

    let policy = format!(
        "version: 1\nhttp:\n  rules:\n    - hosts: [\"{}\"]\n      methods: [POST]\n      paths: [\"/v3/auth/tokens\"]\n      classify: read\n",
        server.address().ip()
    );
    let vm = vm(&policy, ExecMode::ReadOnly);
    let out = eval(
        &vm,
        &format!(
            r#"local r = http.post("{}/v3/auth/tokens", "{{}}") return r.body"#,
            server.uri()
        ),
    )
    .await
    .unwrap();
    assert_eq!(out, "issued");
}

#[tokio::test]
async fn an_undeclared_post_is_still_blocked_under_readonly() {
    let server = MockServer::start().await;
    let policy = format!(
        "version: 1\nhttp:\n  rules:\n    - hosts: [\"{}\"]\n      methods: [POST]\n      paths: [\"/v3/auth/tokens\"]\n      classify: read\n    - hosts: [\"{}\"]\n      methods: [POST]\n      paths: [\"/v3/servers\"]\n",
        server.address().ip(),
        server.address().ip()
    );
    let vm = vm(&policy, ExecMode::ReadOnly);
    let err = eval(
        &vm,
        &format!(
            r#"local r = http.post("{}/v3/servers", "{{}}") return r.body"#,
            server.uri()
        ),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("readonly: http.post blocked"), "got: {err}");
}

#[tokio::test]
async fn readonly_still_blocks_post_when_no_policy_is_loaded() {
    let vm = create_vm_with_policy(
        client(),
        VmOptions {
            mode: ExecMode::ReadOnly,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    let err = eval(
        &vm,
        r#"local r = http.post("http://x.example/y", "{}") return r.body"#,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("readonly: http.post blocked"), "got: {err}");
}

// ------------------------------------------------- redaction and size cap

#[tokio::test]
async fn declared_keys_are_stripped_from_the_response_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/creds"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"user":"a","password":"hunter2"}"#),
        )
        .mount(&server)
        .await;

    let policy = format!(
        "version: 1\nhttp:\n  redact: [password]\n  rules:\n    - hosts: [\"{}\"]\n",
        server.address().ip()
    );
    let vm = vm(&policy, ExecMode::Unrestricted);
    let out = eval(
        &vm,
        &format!(
            r#"local r = http.get("{}/creds") return r.body"#,
            server.uri()
        ),
    )
    .await
    .unwrap();
    assert!(!out.contains("hunter2"), "secret survived: {out}");
    assert!(out.contains("[redacted]"), "not redacted: {out}");
    assert!(out.contains(r#""user":"a""#), "other fields lost: {out}");
}

#[tokio::test]
async fn an_oversized_response_errors_rather_than_truncating() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(4096)))
        .mount(&server)
        .await;

    let policy = format!(
        "version: 1\nhttp:\n  max_response_bytes: 128\n  rules:\n    - hosts: [\"{}\"]\n",
        server.address().ip()
    );
    let vm = vm(&policy, ExecMode::Unrestricted);
    let err = eval(
        &vm,
        &format!(
            r#"local r = http.get("{}/big") return r.body"#,
            server.uri()
        ),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("max_response_bytes"), "got: {err}");
}

// ----------------------------------------------------------- file parsing

#[test]
fn an_unknown_key_is_rejected_rather_than_ignored() {
    let err = Policy::parse("version: 1\nmodules:\n  alow: [assay.json]\n").unwrap_err();
    assert!(err.contains("policy: invalid YAML"), "got: {err}");
}

#[test]
fn an_unsupported_version_is_rejected() {
    let err = Policy::parse("version: 99\n").unwrap_err();
    assert!(err.contains("unsupported version 99"), "got: {err}");
}

#[test]
fn an_unknown_http_method_is_rejected() {
    let err = Policy::parse(
        "version: 1\nhttp:\n  rules:\n    - hosts: [\"a.example.com\"]\n      methods: [FETCH]\n",
    )
    .unwrap_err();
    assert!(err.contains("unknown method 'FETCH'"), "got: {err}");
}

#[test]
fn a_rule_without_hosts_is_rejected() {
    let err = Policy::parse("version: 1\nhttp:\n  rules:\n    - hosts: []\n").unwrap_err();
    assert!(err.contains("needs at least one host"), "got: {err}");
}
