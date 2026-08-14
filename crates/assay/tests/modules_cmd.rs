use serde_json::Value;
use std::process::Command;

fn assay_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_assay"))
}

fn modules_json() -> Value {
    let output = assay_bin().args(["modules", "--json"]).output().unwrap();
    assert!(output.status.success(), "modules --json should exit 0");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "modules --json should emit valid JSON: {e}: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn module_named<'a>(payload: &'a Value, name: &str) -> &'a Value {
    payload["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == name)
        .unwrap_or_else(|| panic!("missing module {name} in {payload}"))
}

#[test]
fn test_modules_exits_zero() {
    let output = assay_bin().arg("modules").output().unwrap();
    assert!(output.status.success(), "modules should exit 0");
}

#[test]
fn test_modules_lists_grafana() {
    let output = assay_bin().arg("modules").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("assay.grafana"),
        "should list assay.grafana: {stdout}"
    );
}

#[test]
fn test_modules_lists_http_builtin() {
    let output = assay_bin().arg("modules").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("http"),
        "should list http builtin: {stdout}"
    );
}

#[test]
fn test_modules_has_header() {
    let output = assay_bin().arg("modules").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("MODULE"),
        "should have MODULE header: {stdout}"
    );
}

#[test]
fn test_modules_lists_vault() {
    let output = assay_bin().arg("modules").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("assay.hashicorp.vault"),
        "should list assay.hashicorp.vault: {stdout}"
    );
    assert!(
        stdout.contains("assay.engine.vault"),
        "should list assay.engine.vault: {stdout}"
    );
}

#[test]
fn test_modules_json_envelope_carries_binary_version() {
    let reported = assay_bin().arg("--version").output().unwrap();
    let reported = String::from_utf8_lossy(&reported.stdout);
    let reported = reported.split_whitespace().last().unwrap();

    let payload = modules_json();
    assert_eq!(
        payload["version"],
        Value::from(reported),
        "consumers cache-bust on this, so it must track the binary: {payload}"
    );
    assert!(
        payload["modules"].as_array().unwrap().len() > 1,
        "expected the discovered modules: {payload}"
    );
}

#[test]
fn test_modules_json_carries_metadata_the_table_drops() {
    let payload = modules_json();
    let grafana = module_named(&payload, "assay.grafana");

    assert_eq!(grafana["source"], "builtin");
    assert!(
        grafana["description"]
            .as_str()
            .is_some_and(|d| !d.is_empty()),
        "grafana: {grafana}"
    );
    assert!(
        grafana["keywords"]
            .as_array()
            .is_some_and(|k| k.contains(&Value::from("grafana"))),
        "keywords drive search and must be exposed: {grafana}"
    );
    assert!(
        grafana["auto_functions"]
            .as_array()
            .is_some_and(|f| !f.is_empty()),
        "auto_functions: {grafana}"
    );

    let quickref = &grafana["quickrefs"][0];
    for field in ["signature", "return_hint", "description"] {
        assert!(
            quickref[field].as_str().is_some_and(|v| !v.is_empty()),
            "quickref.{field} missing: {quickref}"
        );
    }
}

#[test]
fn test_modules_json_carries_env_vars() {
    let payload = modules_json();
    let k8s = module_named(&payload, "assay.k8s");
    assert!(
        k8s["env_vars"]
            .as_array()
            .is_some_and(|e| e.contains(&Value::from("KUBECONFIG"))),
        "env vars tell a caller what the module needs configured: {k8s}"
    );
}

#[test]
fn test_modules_table_unaffected_by_json_support() {
    let output = assay_bin().arg("modules").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with("MODULE"),
        "default output stays the table: {stdout}"
    );
    assert!(
        !stdout.contains("\"modules\""),
        "no JSON without --json: {stdout}"
    );
}

#[test]
fn test_modules_shows_source_column() {
    let output = assay_bin().arg("modules").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("SOURCE"),
        "should have SOURCE header: {stdout}"
    );
    assert!(
        stdout.contains("builtin"),
        "should show builtin source label: {stdout}"
    );
}
