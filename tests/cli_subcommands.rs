use std::process::Command;

fn assay_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_assay"))
}

#[test]
fn help_shows_all_subcommands() {
    let output = assay_bin().arg("--help").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("context"), "help should list context");
    assert!(stdout.contains("exec"), "help should list exec");
    assert!(stdout.contains("modules"), "help should list modules");
    assert!(stdout.contains("run"), "help should list run");
}

#[test]
fn context_help_shows_query_and_limit() {
    let output = assay_bin().args(["context", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("QUERY") || stdout.contains("<QUERY>"),
        "context help should show QUERY: {stdout}"
    );
    assert!(
        stdout.contains("--limit"),
        "context help should show --limit: {stdout}"
    );
}

#[test]
fn exec_help_shows_eval_and_file() {
    let output = assay_bin().args(["exec", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("-e") || stdout.contains("--eval"),
        "exec help should show -e/--eval: {stdout}"
    );
    assert!(
        stdout.contains("FILE") || stdout.contains("[FILE]"),
        "exec help should show FILE: {stdout}"
    );
}

#[test]
fn backward_compat_lua_file() {
    let output = assay_bin()
        .arg("tests/e2e/check_json.lua")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "backward compat lua should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn run_subcommand_lua_file() {
    let output = assay_bin()
        .args(["run", "tests/e2e/check_json.lua"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "run subcommand should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn context_stub_prints_not_implemented() {
    let output = assay_bin()
        .args(["context", "test-query"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "context stub should exit 0");
    assert!(
        stdout.contains("not yet implemented"),
        "context stub should print not yet implemented: {stdout}"
    );
}

#[test]
fn exec_stub_prints_not_implemented() {
    let output = assay_bin()
        .args(["exec", "-e", "print('hello')"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "exec stub should exit 0");
    assert!(
        stdout.contains("not yet implemented"),
        "exec stub should print not yet implemented: {stdout}"
    );
}

#[test]
fn modules_stub_prints_not_implemented() {
    let output = assay_bin().arg("modules").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "modules stub should exit 0");
    assert!(
        stdout.contains("not yet implemented"),
        "modules stub should print not yet implemented: {stdout}"
    );
}

#[test]
fn version_flag_works() {
    let output = assay_bin().arg("--version").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "version should exit 0");
    assert!(
        stdout.contains("assay"),
        "version should contain 'assay': {stdout}"
    );
}


#[test]
fn backward_compat_yaml_file() {
    let output = assay_bin()
        .arg("tests/e2e/check_yaml.lua")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "backward compat yaml should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn backward_compat_toml_file() {
    let output = assay_bin()
        .arg("tests/e2e/check_toml.lua")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "backward compat toml should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn unsupported_extension_fails() {
    let output = assay_bin()
        .arg("nonexistent.txt")
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "unsupported extension should exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unsupported file extension"),
        "error message should mention unsupported extension: {stderr}"
    );
}

#[test]
fn run_subcommand_yaml_file() {
    let output = assay_bin()
        .args(["run", "tests/e2e/check_yaml.lua"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "run subcommand yaml should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}