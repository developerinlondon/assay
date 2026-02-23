use std::process::Command;

fn assay_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_assay"))
}

// --- context subcommand ---

#[test]
fn test_context_vault_finds_module() {
    let output = assay_bin().args(["context", "vault"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "context vault should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("assay.vault"),
        "context vault should find assay.vault, got: {stdout}"
    );
}

#[test]
fn test_context_prometheus_finds_module() {
    let output = assay_bin()
        .args(["context", "prometheus"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "context prometheus should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("assay.prometheus"),
        "context prometheus should find assay.prometheus, got: {stdout}"
    );
}

#[test]
fn test_context_unknown_query_still_exits_zero() {
    let output = assay_bin()
        .args(["context", "xyzzy_nonexistent_module_abc123"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "context with no results should still exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_context_output_has_builtin_section() {
    let output = assay_bin().args(["context", "grafana"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "context grafana should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("Built-in Functions") || stdout.contains("Built-in"),
        "should include built-in functions section, got: {stdout}"
    );
}

// --- modules subcommand ---

#[test]
fn test_modules_lists_all_stdlib() {
    let output = assay_bin().arg("modules").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "modules should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for module in &[
        "assay.grafana",
        "assay.vault",
        "assay.prometheus",
        "assay.k8s",
        "assay.argocd",
    ] {
        assert!(
            stdout.contains(module),
            "modules should list {module}, got: {stdout}"
        );
    }
}

#[test]
fn test_modules_shows_source_column() {
    let output = assay_bin().arg("modules").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "modules should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("builtin"),
        "modules should show 'builtin' source, got: {stdout}"
    );
}

// --- exec subcommand ---

#[test]
fn test_exec_inline_lua() {
    let output = assay_bin()
        .args(["exec", "-e", "log.info('integration test')"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "exec -e should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_exec_inline_assert_passes() {
    let output = assay_bin()
        .args(["exec", "-e", "assert.eq(1 + 1, 2, 'math works')"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "exec with passing assert should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_exec_inline_assert_fails() {
    let output = assay_bin()
        .args(["exec", "-e", "assert.eq(1, 2, 'should fail')"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "exec with failing assert should exit non-zero"
    );
}

#[test]
fn test_exec_inline_json_roundtrip() {
    let output = assay_bin()
        .args([
            "exec",
            "-e",
            "local d = json.parse('{\"k\":\"v\"}'); assert.eq(d.k, 'v')",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "exec with json roundtrip should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_exec_inline_require_stdlib() {
    let output = assay_bin()
        .args([
            "exec",
            "-e",
            "local g = require('assay.grafana'); assert.not_nil(g.client)",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "exec with require should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// --- backward compat / run ---

#[test]
fn test_run_lua_e2e_script() {
    let output = assay_bin()
        .args(["run", "tests/e2e/check_json.lua"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "run tests/e2e/check_json.lua should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_run_v050_e2e_script() {
    let output = assay_bin()
        .args(["run", "tests/e2e/v050_features.lua"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "run tests/e2e/v050_features.lua should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
