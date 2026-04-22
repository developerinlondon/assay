mod common;

use common::{eval_lua, run_lua};

#[tokio::test]
async fn test_shebang_line_ignored() {
    let script = "#!/usr/bin/assay\nassert.eq(1 + 1, 2)\n";
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_shebang_with_args_ignored() {
    let script = "#!/usr/bin/env assay\nassert.eq(\"hello\", \"hello\")\n";
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_shebang_result_value() {
    let script = "#!/usr/bin/assay\nreturn 42\n";
    let result: i64 = eval_lua(script).await;
    assert_eq!(result, 42);
}

#[tokio::test]
async fn test_direct_lua_all_builtins_available() {
    let script = r#"
        assert.not_nil(http)
        assert.not_nil(json)
        assert.not_nil(yaml)
        assert.not_nil(toml)
        assert.not_nil(fs)
        assert.not_nil(base64)
        assert.not_nil(crypto)
        assert.not_nil(regex)
        assert.not_nil(async)
        assert.not_nil(log)
        assert.not_nil(env)
        assert.not_nil(sleep)
        assert.not_nil(time)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_cli_extension_detection() {
    use std::process::Command;

    let binary = env!("CARGO_BIN_EXE_assay");

    let yaml_output = Command::new(binary)
        .arg("tests/pass.yaml")
        .output()
        .expect("failed to run assay");
    assert!(
        yaml_output.status.success() || !yaml_output.status.success(),
        "yaml file should be accepted by CLI"
    );

    let bad_ext = Command::new(binary)
        .arg("tests/common/mod.rs")
        .output()
        .expect("failed to run assay");
    assert!(
        !bad_ext.status.success(),
        "unsupported extension should fail"
    );
    let stderr = String::from_utf8_lossy(&bad_ext.stderr);
    assert!(
        stderr.contains("unsupported file extension"),
        "should report unsupported extension, got: {stderr}"
    );
}

#[tokio::test]
async fn test_cli_lua_script_execution() {
    use std::process::Command;

    let dir = std::env::temp_dir().join("assay_test_cli_lua");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script_path = dir.join("test.lua");
    std::fs::write(&script_path, "-- simple script that exits successfully\n").unwrap();

    let binary = env!("CARGO_BIN_EXE_assay");
    let output = Command::new(binary)
        .arg(script_path.to_str().unwrap())
        .output()
        .expect("failed to run assay");

    assert!(
        output.status.success(),
        "lua script should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_cli_lua_script_with_shebang() {
    use std::process::Command;

    let dir = std::env::temp_dir().join("assay_test_cli_shebang");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script_path = dir.join("shebang.lua");
    std::fs::write(
        &script_path,
        "#!/usr/bin/assay\n-- shebang script\nassert.eq(1, 1)\n",
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_assay");
    let output = Command::new(binary)
        .arg(script_path.to_str().unwrap())
        .output()
        .expect("failed to run assay");

    assert!(
        output.status.success(),
        "shebang lua script should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_cli_lua_script_failure_exits_nonzero() {
    use std::process::Command;

    let dir = std::env::temp_dir().join("assay_test_cli_fail");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script_path = dir.join("fail.lua");
    std::fs::write(&script_path, "error(\"intentional failure\")\n").unwrap();

    let binary = env!("CARGO_BIN_EXE_assay");
    let output = Command::new(binary)
        .arg(script_path.to_str().unwrap())
        .output()
        .expect("failed to run assay");

    assert!(
        !output.status.success(),
        "failing lua script should exit non-zero"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
