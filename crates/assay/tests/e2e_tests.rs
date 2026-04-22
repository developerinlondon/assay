use std::process::Command;

#[test]
fn test_e2e_builtins_check() {
    let binary = env!("CARGO_BIN_EXE_assay");
    let output = Command::new(binary)
        .arg("tests/e2e/builtins-check.yaml")
        .output()
        .expect("failed to run assay");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "E2E builtins check failed:\nstdout: {stdout}\nstderr: {stderr}"
    );

    let result: serde_json::Value =
        serde_json::from_str(&stdout).expect("invalid JSON output from assay");
    assert_eq!(
        result["passed"], true,
        "not all checks passed:\n{stdout}\nstderr: {stderr}"
    );

    let checks = result["checks"]
        .as_array()
        .expect("checks should be an array");
    assert_eq!(checks.len(), 8, "expected 8 checks, got {}", checks.len());

    for check in checks {
        assert_eq!(
            check["passed"],
            true,
            "check {:?} failed: {}",
            check["name"],
            check["message"].as_str().unwrap_or("(no message)")
        );
    }
}
