use std::sync::Arc;

use assay::lua::policy::Policy;
use assay::lua::{ExecMode, VmOptions, create_vm_with_policy};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SECRET: &str = "s3cr3t-value-not-in-lua";

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap()
}

fn vm(policy_yaml: &str) -> mlua::Lua {
    unsafe {
        std::env::set_var("ASSAY_CRED_TEST_PASSWORD", SECRET);
        std::env::set_var("ASSAY_CRED_TEST_USER", "svc-reader");
    }
    let policy = Arc::new(Policy::parse(policy_yaml).expect("policy parses"));
    create_vm_with_policy(
        client(),
        VmOptions {
            mode: ExecMode::Unrestricted,
            ..Default::default()
        },
        Some(policy),
    )
    .unwrap()
}

fn creds_section() -> String {
    "credentials:\n  inventory-ro:\n    username: ASSAY_CRED_TEST_USER\n    password: ASSAY_CRED_TEST_PASSWORD\n".to_string()
}

async fn eval(vm: &mlua::Lua, script: &str) -> mlua::Result<String> {
    vm.load(script).eval_async::<String>().await
}

#[tokio::test]
async fn a_handle_never_exposes_the_secret_to_lua() {
    let vm = vm(&format!("version: 1\n{}", creds_section()));
    let out = eval(
        &vm,
        r#"local c = credential.get("inventory-ro")
           return c.password .. "|" .. tostring(c.password):upper() .. "|" .. json.encode(c)"#,
    )
    .await
    .unwrap();
    assert!(!out.contains(SECRET), "the secret leaked into Lua: {out}");
    assert!(
        !out.to_lowercase().contains("s3cr3t"),
        "the secret leaked into Lua: {out}"
    );
}

#[tokio::test]
async fn an_undeclared_credential_is_an_error_not_an_empty_handle() {
    let vm = vm(&format!("version: 1\n{}", creds_section()));
    let err = eval(&vm, r#"local c = credential.get("other") return "ok""#)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("'other' is not declared in the policy"),
        "got: {err}"
    );
}

#[tokio::test]
async fn the_real_secret_reaches_an_allowed_target() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v3/auth/tokens"))
        .and(body_string_contains(SECRET))
        .respond_with(ResponseTemplate::new(201).set_body_string("issued"))
        .mount(&server)
        .await;

    let policy = format!(
        "version: 1\nhttp:\n  rules:\n    - hosts: [\"{}\"]\n{}",
        server.address().ip(),
        creds_section()
    );
    let vm = vm(&policy);
    let out = eval(
        &vm,
        &format!(
            r#"local c = credential.get("inventory-ro")
               local r = http.post("{}/v3/auth/tokens", {{ user = c.username, pass = c.password }})
               return r.body"#,
            server.uri()
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        out, "issued",
        "the substituted body did not match the server expectation"
    );
}

#[tokio::test]
async fn a_handle_in_a_url_is_refused() {
    let policy = format!("version: 1\n{}", creds_section());
    let vm = vm(&policy);
    let err = eval(
        &vm,
        r#"local c = credential.get("inventory-ro")
           local r = http.get("http://example.com/?k=" .. c.password)
           return r.body"#,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("cannot be used in a URL"), "got: {err}");
}

#[tokio::test]
async fn env_allowlist_and_credentials_compose() {
    let policy = format!("version: 1\nenv:\n  allow: []\n{}", creds_section());
    let vm = vm(&policy);
    let out = eval(
        &vm,
        r#"return tostring(env.get("ASSAY_CRED_TEST_PASSWORD"))"#,
    )
    .await
    .unwrap();
    assert_eq!(
        out, "nil",
        "the credential's backing env key must stay unreadable"
    );
}

#[tokio::test]
async fn no_credential_table_exists_without_a_policy() {
    let vm = create_vm_with_policy(client(), VmOptions::default(), None).unwrap();
    let out = eval(&vm, r#"return type(credential)"#).await.unwrap();
    assert_eq!(out, "nil");
}
