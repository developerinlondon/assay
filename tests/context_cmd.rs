use std::process::Command;

fn assay_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_assay"))
}

#[test]
fn test_context_grafana_exits_zero() {
    let output = assay_bin().args(["context", "grafana"]).output().unwrap();
    assert!(output.status.success(), "context grafana should exit 0");
}

#[test]
fn test_context_grafana_outputs_markdown_header() {
    let output = assay_bin().args(["context", "grafana"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("# Assay Module Context"),
        "should output markdown header, got: {stdout}"
    );
}

#[test]
fn test_context_grafana_finds_module() {
    let output = assay_bin().args(["context", "grafana"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("assay.grafana"),
        "should find assay.grafana module, got: {stdout}"
    );
}

#[test]
fn test_context_limit_flag() {
    let output = assay_bin()
        .args(["context", "a", "--limit", "2"])
        .output()
        .unwrap();
    assert!(output.status.success(), "context with --limit should exit 0");
}
