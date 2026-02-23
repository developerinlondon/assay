use std::process::Command;

fn assay_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_assay"))
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
        stdout.contains("assay.vault"),
        "should list assay.vault: {stdout}"
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
