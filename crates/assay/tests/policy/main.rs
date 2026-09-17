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

// -------------------------------------------------------------------- dns

// None of these reach a socket: the refusal happens before the query is built,
// and the spec in the last is rejected before one could be sent.

#[tokio::test]
async fn a_caller_chosen_nameserver_is_refused_under_a_policy() {
    let vm = vm("version: 1\n", ExecMode::Unrestricted);
    let err = eval(
        &vm,
        r#"dns.lookup("example.com", "A", { server = "127.0.0.1:5353" }) return "resolved""#,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("dns.lookup: opts.server is not allowed while a policy is installed"),
        "got: {err}"
    );
}

#[tokio::test]
async fn a_dnsbl_check_cannot_pick_its_own_nameserver_either() {
    let vm = vm("version: 1\n", ExecMode::Unrestricted);
    let err = eval(
        &vm,
        r#"dns.dnsbl("example.com", "bl.example.net", { server = "127.0.0.1:5353" })
           return "asked""#,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("dns.dnsbl: opts.server is not allowed while a policy is installed"),
        "got: {err}"
    );
}

#[tokio::test]
async fn without_a_policy_the_server_option_is_taken_rather_than_refused() {
    // An unparseable server proves the option was read at all: the refusal
    // above is the policy's doing, not the option being unsupported.
    let vm = unpoliced();
    let err = eval(
        &vm,
        r#"dns.lookup("example.com", "A", { server = "resolver.example.com" }) return "resolved""#,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("is not a nameserver address"), "got: {err}");
    assert!(!err.contains("not allowed"), "refused instead: {err}");
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

// ---------------------------------------------------------------- globals

/// The reachability question the whole feature exists for: a host confining a
/// script to one HTTP origin needs assay's own globals gone, and before this
/// only Lua stdlib names could be removed.
#[tokio::test]
async fn a_policy_globals_list_removes_assay_builtins() {
    let vm = vm(
        "version: 1\nglobals:\n  block: [fs, db, dns, ws]\n",
        ExecMode::ReadOnly,
    );
    let out = eval(
        &vm,
        r#"
        local gone = {}
        for _, name in ipairs({"fs", "db", "dns", "ws"}) do
            if _G[name] ~= nil then gone[#gone + 1] = name end
        end
        if #gone > 0 then return "still there: " .. table.concat(gone, ",") end
        -- a global the policy did not name is untouched
        if type(http) ~= "table" then return "http went missing" end
        return "removed"
        "#,
    )
    .await
    .unwrap();
    assert_eq!(out, "removed");
}

/// Removing the global is not the same lever as the module allowlist, and
/// naming one must not disturb the other.
#[tokio::test]
async fn blocking_a_global_leaves_require_governed_by_the_module_allowlist() {
    let vm = vm(
        "version: 1\nglobals:\n  block: [fs]\nmodules:\n  allow: [assay.url]\n",
        ExecMode::Unrestricted,
    );
    assert!(
        eval(&vm, r#"return type(fs)"#).await.unwrap() == "nil",
        "the global should be gone"
    );
    let allowed = eval(
        &vm,
        r#"local u = require("assay.url") return type(u.encode)"#,
    )
    .await
    .unwrap();
    assert_eq!(allowed, "function", "an allowed module still loads");
    let err = eval(&vm, r#"require("assay.openstack") return "loaded""#)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("not in the allowed set"),
        "the allowlist still refuses: {err}"
    );
}

#[tokio::test]
async fn a_policy_without_a_globals_list_removes_nothing() {
    let vm = vm("version: 1\n", ExecMode::Unrestricted);
    let out = eval(&vm, r#"return type(fs) .. "," .. type(db)"#)
        .await
        .unwrap();
    assert_eq!(out, "table,table");
}

#[test]
fn a_globals_list_parses_and_is_readable() {
    let policy =
        Policy::parse("version: 1\nglobals:\n  block: [fs, os.execute]\n").expect("parses");
    assert_eq!(policy.blocked_globals(), ["fs", "os.execute"]);
    let empty = Policy::parse("version: 1\n").expect("parses");
    assert!(empty.blocked_globals().is_empty());
}

/// `require` reads `package.loaded` before any searcher, so clearing `_G.io`
/// alone left `require("io")` handing back the very library the policy named.
#[tokio::test]
async fn a_blocked_bare_name_is_gone_from_require_too() {
    let vm = vm(
        "version: 1\nglobals:\n  block: [io]\n",
        ExecMode::Unrestricted,
    );
    let out = eval(
        &vm,
        r#"
        if io ~= nil then return "_G.io survived" end
        if package.loaded["io"] ~= nil then return "package.loaded.io survived" end
        local ok, err = pcall(require, "io")
        if ok then return "require('io') still returns it" end
        return "gone"
        "#,
    )
    .await
    .unwrap();
    assert_eq!(out, "gone");
}

/// assay replaces `_G.os` with its own table, which never had `execute`, so a
/// dotted path that only walked globals cleared nothing while Lua's real `os`
/// stayed reachable behind `require`.
#[tokio::test]
async fn a_blocked_dotted_path_reaches_the_table_behind_require() {
    let vm = vm(
        "version: 1\nglobals:\n  block: [os.execute, os.remove]\n",
        ExecMode::Unrestricted,
    );
    let out = eval(
        &vm,
        r#"
        local os_lib = require("os")
        if os_lib.execute ~= nil then return "os.execute survived" end
        if os_lib.remove ~= nil then return "os.remove survived" end
        -- the rest of the table is untouched, and so is assay's own os
        if type(os_lib.time) ~= "function" then return "os.time was collateral" end
        if type(os.date) ~= "function" then return "assay's os was damaged" end
        return "cleared"
        "#,
    )
    .await
    .unwrap();
    assert_eq!(out, "cleared");
}

#[test]
fn a_bare_globals_list_is_rejected_rather_than_ignored() {
    let err = Policy::parse("version: 1\nglobals: [fs]\n").unwrap_err();
    assert!(err.contains("invalid YAML"), "got: {err}");
}

#[test]
fn an_unknown_key_under_globals_is_rejected() {
    let err = Policy::parse("version: 1\nglobals:\n  allow: [fs]\n").unwrap_err();
    assert!(err.contains("invalid YAML"), "got: {err}");
}

/// `env.allow` decides what the environment shows, and `os.getenv` reads the
/// same environment by another name. assay's `os` has no `getenv`, so before
/// this an allowlisted VM handed out every variable it held through
/// `require("os")`.
#[tokio::test]
async fn os_getenv_respects_the_env_allowlist() {
    // The api-server guide's own policy, with its own key names: it allows
    // OS_PROJECT_NAME and holds OS_PASSWORD as a credential the script must
    // not read. That page says the caller "cannot read the credential", and
    // until `os.getenv` answered through the allowlist that sentence was false.
    //
    // SAFETY: the values are this test's own, and the assertion is about what
    // the policy shows rather than about the process environment changing.
    unsafe {
        std::env::set_var("OS_PROJECT_NAME", "inventory");
        std::env::set_var("OS_PASSWORD", "s3cret");
    }
    let vm = vm(
        "version: 1\nenv:\n  allow: [OS_PROJECT_NAME]\n",
        ExecMode::Unrestricted,
    );
    let out = eval(
        &vm,
        r#"
        local os_lib = require("os")
        return tostring(os_lib.getenv("OS_PROJECT_NAME")) .. "/"
            .. tostring(os_lib.getenv("OS_PASSWORD")) .. "/"
            .. tostring(env.get("OS_PASSWORD"))
        "#,
    )
    .await
    .unwrap();
    unsafe {
        std::env::remove_var("OS_PROJECT_NAME");
        std::env::remove_var("OS_PASSWORD");
    }
    // The allowed key reads through; the credential is indistinguishable from
    // unset, exactly as `env.get` reports it.
    assert_eq!(out, "inventory/nil/nil");
}

/// Blocking a table must never be a way to get an ungated one.
#[tokio::test]
async fn blocking_io_under_readonly_yields_no_ungated_handle() {
    let vm = vm("version: 1\nglobals:\n  block: [io]\n", ExecMode::ReadOnly);
    let out = eval(
        &vm,
        r#"
        if io ~= nil then return "_G.io survived" end
        if package.loaded["io"] ~= nil then return "package.loaded.io survived" end
        if pcall(require, "io") then return "require handed it back" end
        return "no handle"
        "#,
    )
    .await
    .unwrap();
    assert_eq!(out, "no handle");
}

/// The other half of the same rule: a table the policy does NOT block stays
/// gated by the mode, rather than being skipped because something removed it.
#[tokio::test]
async fn an_unblocked_table_is_still_gated_by_readonly() {
    let vm = vm("version: 1\nglobals:\n  block: [db]\n", ExecMode::ReadOnly);
    let out = eval(
        &vm,
        r#"
        local io_lib = require("io")
        local popen_ok = pcall(io_lib.popen, "true")
        local write_ok = pcall(io_lib.open, "/tmp/assay-policy-probe", "w")
        if popen_ok then return "popen ran" end
        if write_ok then return "open-for-write ran" end
        if db ~= nil then return "db survived" end
        return "gated"
        "#,
    )
    .await
    .unwrap();
    assert_eq!(out, "gated");
}
