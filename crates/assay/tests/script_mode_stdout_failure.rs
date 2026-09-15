// Script mode's answer is whatever the script printed, so a stdout write that
// fails and is never noticed hands the caller exit 0 with no output — the one
// failure that is indistinguishable from a script which printed nothing. Lua's
// `print` goes through C stdio and ignores what `fwrite`/`fflush` return, so
// the binary has to check the stream itself.

use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::NamedTempFile;

fn assay_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_assay"))
}

fn write_lua(body: &str) -> NamedTempFile {
    let mut f = NamedTempFile::with_suffix(".lua").unwrap();
    f.write_all(body.as_bytes()).unwrap();
    f
}

/// `/dev/full` is a Linux device that accepts an `open` and fails every write
/// with `ENOSPC` — a full disk without needing one.
fn dev_full() -> Option<std::fs::File> {
    if !std::path::Path::new("/dev/full").exists() {
        return None;
    }
    std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .ok()
}

#[test]
fn run_fails_loudly_when_stdout_is_full() {
    let Some(full) = dev_full() else {
        return;
    };
    let f = write_lua(r#"print(json.encode({ ok = true }))"#);

    let out = assay_bin()
        .arg("run")
        .arg(f.path())
        .stdout(full)
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 on a failed stdout write, stderr={stderr}"
    );
    assert!(stderr.contains("ERROR"), "stderr={stderr}");
}

#[test]
fn exec_fails_loudly_when_stdout_is_full() {
    let Some(full) = dev_full() else {
        return;
    };

    let out = assay_bin()
        .arg("exec")
        .arg("-e")
        .arg(r#"print(json.encode({ ok = true }))"#)
        .stdout(full)
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 on a failed stdout write, stderr={stderr}"
    );
    assert!(stderr.contains("ERROR"), "stderr={stderr}");
}

#[test]
fn run_still_succeeds_when_stdout_works() {
    let f = write_lua(
        r#"
        print(json.encode({ ok = true }))
    "#,
    );

    let out = assay_bin().arg("run").arg(f.path()).output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "stderr={stderr} stdout={stdout}");
    assert!(stdout.contains(r#""ok""#), "stdout={stdout}");
}

/// A reader that walks away — `assay run script.lua | head` — stays exit 0,
/// like `assay completion` and `assay modules --json` already do. Deterministic
/// either way round: if the writes land in the pipe buffer nothing failed at
/// all, and if they hit `EPIPE` the carve-out has to recognise it.
#[test]
fn run_succeeds_when_the_reader_goes_away() {
    let f = write_lua(
        r#"
        local line = string.rep("x", 1024)
        for _ = 1, 1024 do print(line) end
    "#,
    );

    let mut child = assay_bin()
        .arg("run")
        .arg(f.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // Close the read end while the script is still printing.
    drop(child.stdout.take());

    let status = child.wait().unwrap();
    assert!(
        status.success(),
        "a dead reader must stay a success, got exit {:?}",
        status.code()
    );
}
