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
fn test_context_cli_includes_builtins_by_default() {
    let output = assay_bin().args(["context", "grafana"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("## Built-in Functions"),
        "a human reading the CLI output still gets the reference: {stdout}"
    );
    assert!(
        stdout.contains("http.get(url, opts?)"),
        "builtins body missing: {stdout}"
    );
}

#[test]
fn test_context_no_builtins_omits_the_block() {
    let output = assay_bin()
        .args(["context", "grafana", "--no-builtins"])
        .output()
        .unwrap();
    assert!(output.status.success(), "--no-builtins should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("assay.grafana"),
        "module docs still rendered: {stdout}"
    );
    assert!(
        !stdout.contains("Built-in Functions"),
        "builtins block should be gone: {stdout}"
    );
}

#[test]
fn test_context_limit_flag() {
    let output = assay_bin()
        .args(["context", "a", "--limit", "2"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "context with --limit should exit 0"
    );
}

/// Acceptance for `assay.jev`: an agent asking the MCP surface for "jev" has to
/// get the module doc back, and the three builders are the whole of the API it
/// can see — `assay context` renders the description, the env var and the
/// quickref lines, and nothing else from the doc block.
#[test]
fn test_context_jev_returns_the_module_doc() {
    let output = assay_bin().args(["context", "jev"]).output().unwrap();
    assert!(output.status.success(), "context jev should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("assay.jev"),
        "should find it, got: {stdout}"
    );
    assert!(
        stdout.contains("TYPESAFE_API_KEY"),
        "the env var it needs is part of the doc: {stdout}"
    );
    for wanted in [
        "M.client(opts?)",
        "c:system_one(state, questions, opts?)",
        "M.choice(instructions, criteria)",
        "M.score(instructions, levels)",
        "M.noul(instructions, criteria?)",
    ] {
        assert!(
            stdout.contains(wanted),
            "quickref for {wanted} did not render — a line missing ` -> ` or ` | ` is dropped silently: {stdout}"
        );
    }
}
