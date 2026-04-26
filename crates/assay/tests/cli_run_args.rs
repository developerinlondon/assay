// Verifies that `assay run script.lua -- arg1 arg2` populates Lua's
// `arg` global the way a normal `lua` interpreter would: arg[0] is the
// script path, arg[1..] are the user-passed positional values.

use std::io::Write;
use std::process::Command;
use tempfile::NamedTempFile;

fn assay_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_assay"))
}

fn write_lua(body: &str) -> NamedTempFile {
    let mut f = NamedTempFile::with_suffix(".lua").unwrap();
    f.write_all(body.as_bytes()).unwrap();
    f
}

#[test]
fn run_passes_positional_args_to_arg_global() {
    // Note: assay's `assert` is a table (`assert.eq`, …), so the
    // script asserts via `error()` on mismatch.
    let f = write_lua(
        r#"
        if type(arg) ~= "table" then error("arg must be a table") end
        assert.eq(arg[1], "--email")
        assert.eq(arg[2], "alice@example.com")
        assert.eq(arg[3], "--password")
        assert.eq(arg[4], "hunter2")
        assert.eq(arg[5], nil)
        print("arg-ok")
    "#,
    );

    let out = assay_bin()
        .arg("run")
        .arg(f.path())
        .arg("--")
        .arg("--email")
        .arg("alice@example.com")
        .arg("--password")
        .arg("hunter2")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "exit code {} stderr={stderr} stdout={stdout}",
        out.status.code().unwrap_or(-1)
    );
    assert!(stdout.contains("arg-ok"), "stdout: {stdout}");
}

#[test]
fn run_passes_arg0_as_script_path() {
    let f = write_lua(
        r#"
        if type(arg) ~= "table" then error("arg must be a table") end
        if type(arg[0]) ~= "string" then error("arg[0] must be a string") end
        if #arg[0] == 0 then error("arg[0] must be non-empty") end
        print("arg0=" .. arg[0])
    "#,
    );

    let out = assay_bin().arg("run").arg(f.path()).output().unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("arg0="), "stdout: {stdout}");
}

#[test]
fn run_with_no_extra_args_yields_empty_arg_table() {
    let f = write_lua(
        r#"
        if type(arg) ~= "table" then error("arg must be a table") end
        assert.eq(arg[1], nil)
        assert.eq(arg[2], nil)
        print("empty-ok")
    "#,
    );
    let out = assay_bin().arg("run").arg(f.path()).output().unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("empty-ok"), "stdout: {stdout}");
}
