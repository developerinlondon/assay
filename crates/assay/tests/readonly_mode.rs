// Read-only execution mode: mutating builtins stay registered but
// raise `readonly: <name> blocked (write operations are disabled in
// read-only mode)`, while read paths keep working. Activated per-VM
// via `create_vm_with_paths(..., readonly = true)`, process-wide via
// ASSAY_READONLY=1|true, or per-invocation via the global `--readonly`
// CLI flag.

// ASSAY_READONLY is read at VM construction time (process-wide).
// cargo's default test harness runs tests on threads in the same
// process, so any test that builds a VM races against any test that
// mutates that env var. ENV_LOCK serializes ALL VM creation in this
// file, held only across the synchronous `create_vm*` call — never
// across an `await`.

use serde_json::Value as JsonValue;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

static ENV_LOCK: Mutex<()> = Mutex::new(());

const BLOCKED_MSG: &str = "blocked (write operations are disabled in read-only mode)";

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap()
}

fn make_vm(readonly: bool) -> mlua::Lua {
    let _g = ENV_LOCK.lock().unwrap();
    assay::lua::create_vm_with_paths(http_client(), None, readonly).unwrap()
    // _g dropped here, before any caller awaits.
}

fn make_vm_with_env(value: &str) -> mlua::Lua {
    let _g = ENV_LOCK.lock().unwrap();
    // SAFETY: ENV_LOCK serialises every test in this file that mutates
    // ASSAY_READONLY or constructs a VM that reads it.
    unsafe {
        std::env::set_var(assay::lua::READONLY_ENV, value);
    }
    let vm = assay::lua::create_vm(http_client());
    unsafe {
        std::env::remove_var(assay::lua::READONLY_ENV);
    }
    vm.unwrap()
    // _g dropped here.
}

async fn run(script: &str, vm: mlua::Lua) {
    let bridged = assay::lua::async_bridge::strip_shebang(script);
    vm.load(bridged).exec_async().await.unwrap();
}

// Spawned children must not inherit ASSAY_READONLY from the test
// process: the in-process env-activation tests set it (under ENV_LOCK)
// while CLI tests run on sibling threads, and Command inherits the
// parent env at spawn time. Tests that want env activation re-add it
// explicitly.
fn assay_bin() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_assay"));
    cmd.env_remove(assay::lua::READONLY_ENV);
    cmd
}

fn write_file(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
    let file_path = dir.join(name);
    let mut f = std::fs::File::create(&file_path).unwrap();
    f.write_all(body.as_bytes()).unwrap();
    file_path
}

// ── in-process VM behaviour ────────────────────────────────────────────

#[tokio::test]
async fn http_get_allowed_in_readonly() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "ok"
        })))
        .mount(&server)
        .await;

    let vm = make_vm(true);
    let script = format!(
        r#"
        local resp = http.get("{}/health")
        assert.eq(resp.status, 200)
        local body = json.parse(resp.body)
        assert.eq(body.status, "ok")
        "#,
        server.uri()
    );
    run(&script, vm).await;
}

#[tokio::test]
async fn http_write_verbs_blocked_in_readonly() {
    let vm = make_vm(true);
    let script = r#"
        local ok, err = pcall(http.post, "http://127.0.0.1:1/x", "{}")
        assert.eq(ok, false)
        assert.contains(
            tostring(err),
            "readonly: http.post blocked (write operations are disabled in read-only mode)"
        )

        for _, name in ipairs({ "put", "patch", "delete", "serve", "serve_with_extra", "download" }) do
            local ok2, err2 = pcall(http[name])
            assert.eq(ok2, false)
            assert.contains(tostring(err2), "readonly: http." .. name .. " blocked")
        end

        -- read helpers keep their surface
        assert.eq(type(http.get), "function")
        assert.eq(type(http.client), "function")
    "#;
    run(script, vm).await;
}

#[tokio::test]
async fn http_client_wrapper_get_allowed_post_blocked_in_readonly() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let vm = make_vm(true);
    let script = format!(
        r#"
        local c = http.client({{ timeout = 2 }})
        local resp = c:get("{}/health")
        assert.eq(resp.status, 200)
        "#,
        server.uri()
    );
    run(&script, vm).await;

    let vm = make_vm(true);
    let err = vm
        .load(
            r#"
            local c = http.client({ timeout = 2 })
            c:post("http://127.0.0.1:1/x", "{}")
            "#,
        )
        .exec_async()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("readonly: http.post blocked"), "got: {msg}");
}

#[tokio::test]
async fn shell_and_process_blocked_in_readonly() {
    let vm = make_vm(true);
    let script = r#"
        local ok, err = pcall(shell.exec, "echo hi")
        assert.eq(ok, false)
        assert.contains(tostring(err), "readonly: shell.exec blocked")

        for _, name in ipairs({ "list", "is_running", "kill", "spawn", "wait", "spawn_pty" }) do
            local ok2, err2 = pcall(process[name])
            assert.eq(ok2, false)
            assert.contains(tostring(err2), "readonly: process." .. name .. " blocked")
        end
    "#;
    run(script, vm).await;
}

#[tokio::test]
async fn fs_writes_blocked_reads_allowed_in_readonly() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("data.txt");
    std::fs::write(&file_path, "hello readonly").unwrap();

    let vm = make_vm(true);
    let script = format!(
        r#"
        assert.eq(fs.read("{path}"), "hello readonly")
        assert.eq(fs.exists("{path}"), true)
        assert.not_nil(fs.stat("{path}"))
        assert.not_nil(fs.list("{dir}"))

        local blocked = {{
            "write", "write_bytes", "remove", "rename", "copy",
            "chmod", "mkdir", "tempdir", "sub_in_file",
        }}
        for _, name in ipairs(blocked) do
            local ok, err = pcall(fs[name], "{path}", "x")
            assert.eq(ok, false)
            assert.contains(tostring(err), "readonly: fs." .. name .. " blocked")
        end
        "#,
        path = file_path.display(),
        dir = dir.path().display()
    );
    run(&script, vm).await;
    assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "hello readonly");
}

#[tokio::test]
async fn env_get_allowed_env_set_blocked_in_readonly() {
    let vm = make_vm(true);
    let script = r#"
        assert.eq(type(env.get("HOME")), "string")
        assert.not_nil(env.list())

        local ok, err = pcall(env.set, "ASSAY_READONLY_TEST_VAR", "1")
        assert.eq(ok, false)
        assert.contains(tostring(err), "readonly: env.set blocked")
    "#;
    run(script, vm).await;
}

#[cfg(feature = "db")]
#[tokio::test]
async fn db_execute_blocked_query_stays_in_readonly() {
    let vm = make_vm(true);
    let script = r#"
        assert.eq(type(db.connect), "function")
        assert.eq(type(db.query), "function")
        assert.eq(type(db.close), "function")

        local ok, err = pcall(db.execute)
        assert.eq(ok, false)
        assert.contains(tostring(err), "readonly: db.execute blocked")
    "#;
    run(script, vm).await;
}

#[tokio::test]
async fn ws_connect_blocked_in_readonly() {
    let vm = make_vm(true);
    let script = r#"
        local ok, err = pcall(ws.connect, "ws://127.0.0.1:1/socket")
        assert.eq(ok, false)
        assert.contains(tostring(err), "readonly: ws.connect blocked")
    "#;
    run(script, vm).await;
}

#[tokio::test]
async fn system_mutators_blocked_reads_stay_in_readonly() {
    let vm = make_vm(true);
    let script = r#"
        local blocked = {
            { oci, "oci", { "copy", "tag", "mutate" } },
            { systemd, "systemd", {
                "start", "stop", "restart", "reload", "unit_action",
                "machine_start", "machine_poweroff", "machine_reboot",
                "machine_terminate", "machine_exec",
            } },
            { apt, "apt", { "update", "install", "remove", "add_source" } },
            { machinectl, "machinectl", { "pull_tar", "pull_raw", "remove", "clone" } },
            { tar, "tar", { "create", "extract" } },
            { compress, "compress", { "untar" } },
        }
        -- some systemd entries are Linux-only; assert every function
        -- that exists on this platform raises the readonly error
        for _, entry in ipairs(blocked) do
            local tbl, tbl_name, names = entry[1], entry[2], entry[3]
            for _, name in ipairs(names) do
                if tbl[name] ~= nil then
                    local ok, err = pcall(tbl[name])
                    assert.eq(ok, false)
                    assert.contains(tostring(err), "readonly: " .. tbl_name .. "." .. name .. " blocked")
                end
            end
        end

        -- read surfaces stay intact
        assert.eq(type(systemd.list_units), "function")
        assert.eq(type(systemd.unit_status), "function")
        assert.eq(type(systemd.journal), "function")
        assert.eq(type(apt.query), "function")
        assert.eq(type(apt.list_installed), "function")
        assert.eq(type(tar.list), "function")
        assert.eq(type(compress.gunzip), "function")
        assert.eq(type(disk.usage), "function")
    "#;
    run(script, vm).await;
}

#[tokio::test]
async fn lua_stdlib_write_paths_blocked_in_readonly() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("readable.txt");
    std::fs::write(&file_path, "line").unwrap();

    let vm = make_vm(true);
    let script = format!(
        r#"
        local ok, err = pcall(io.popen, "echo hi")
        assert.eq(ok, false)
        assert.contains(tostring(err), "readonly: io.popen blocked")

        for _, mode in ipairs({{ "w", "a", "r+" }}) do
            local ok, err = pcall(io.open, "{path}", mode)
            assert.eq(ok, false)
            assert.contains(tostring(err), "readonly: io.open blocked")
        end

        local ok_out, err_out = pcall(io.output, "{path}")
        assert.eq(ok_out, false)
        assert.contains(tostring(err_out), "readonly: io.output blocked")

        -- read mode stays open
        local f = io.open("{path}", "r")
        assert.eq(f:read("l"), "line")
        f:close()
        "#,
        path = file_path.display()
    );
    run(&script, vm).await;
}

#[tokio::test]
async fn mode_off_writes_work() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/submit"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("out.txt");

    let vm = make_vm(false);
    let script = format!(
        r#"
        local resp = http.post("{uri}/submit", "{{}}")
        assert.eq(resp.status, 201)
        fs.write("{path}", "written")
        assert.eq(fs.read("{path}"), "written")
        env.set("ASSAY_READONLY_OFF_TEST", "1")
        assert.eq(env.get("ASSAY_READONLY_OFF_TEST"), "1")
        "#,
        uri = server.uri(),
        path = file_path.display()
    );
    run(&script, vm).await;
    assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "written");
}

#[tokio::test]
async fn env_var_activates_readonly() {
    for value in ["1", "true"] {
        let vm = make_vm_with_env(value);
        let script = r#"
            local ok, err = pcall(fs.write, "/tmp/assay-readonly-env-test", "x")
            assert.eq(ok, false)
            assert.contains(tostring(err), "readonly: fs.write blocked")
        "#;
        run(script, vm).await;
    }
}

#[tokio::test]
async fn env_var_other_values_do_not_activate_readonly() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("off.txt");

    let vm = make_vm_with_env("0");
    let script = format!(
        r#"
        fs.write("{path}", "not readonly")
        assert.eq(fs.read("{path}"), "not readonly")
        "#,
        path = file_path.display()
    );
    run(&script, vm).await;
}

// ── CLI surface ────────────────────────────────────────────────────────

const CLI_PROBE_SCRIPT: &str = r#"
    local ok, err = pcall(fs.write, "/tmp/assay-readonly-cli-test", "x")
    assert.eq(ok, false)
    assert.contains(tostring(err), "readonly: fs.write blocked")
    print("readonly-ok")
"#;

#[test]
fn cli_readonly_flag_blocks_writes() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_file(dir.path(), "probe.lua", CLI_PROBE_SCRIPT);

    for args in [
        vec!["run", "--readonly", script.to_str().unwrap()],
        vec!["--readonly", script.to_str().unwrap()],
    ] {
        let out = assay_bin().args(&args).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "args={args:?} stderr={stderr}");
        assert!(stdout.contains("readonly-ok"), "args={args:?} stdout={stdout}");
    }
}

#[test]
fn cli_env_var_blocks_writes() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_file(dir.path(), "probe.lua", CLI_PROBE_SCRIPT);

    let out = assay_bin()
        .args(["run", script.to_str().unwrap()])
        .env("ASSAY_READONLY", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stderr={stderr}");
    assert!(stdout.contains("readonly-ok"), "stdout={stdout}");
}

#[test]
fn cli_tool_envelope_reports_readonly() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_file(
        dir.path(),
        "tool.lua",
        r#"return { mode = env.get("ASSAY_MODE") }"#,
    );

    let out = assay_bin()
        .args(["run", "--mode", "tool", "--readonly", script.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let envelope: JsonValue = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(envelope["ok"], JsonValue::Bool(true));
    assert_eq!(envelope["status"], "ok");
    assert_eq!(envelope["readonly"], JsonValue::Bool(true));
    assert_eq!(envelope["output"]["mode"], "tool");
}

#[test]
fn cli_tool_envelope_omits_readonly_when_off() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_file(dir.path(), "tool.lua", r#"return { done = true }"#);

    let out = assay_bin()
        .args(["run", "--mode", "tool", script.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let envelope: JsonValue = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(envelope["status"], "ok");
    assert!(envelope.get("readonly").is_none(), "envelope: {envelope}");
}

#[test]
fn cli_tool_error_envelope_reports_readonly() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_file(dir.path(), "tool.lua", r#"shell.exec("echo hi")"#);

    let out = assay_bin()
        .args(["run", "--mode", "tool", "--readonly", script.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let envelope: JsonValue = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(envelope["ok"], JsonValue::Bool(false));
    assert_eq!(envelope["status"], "error");
    assert_eq!(envelope["readonly"], JsonValue::Bool(true));
    let error = envelope["error"].as_str().unwrap();
    assert!(error.contains("readonly: shell.exec blocked"), "error: {error}");
    assert!(error.contains(BLOCKED_MSG), "error: {error}");
}

#[test]
fn cli_modules_reports_readonly() {
    let out = assay_bin().args(["modules", "--readonly"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("read-only mode active"), "stdout: {stdout}");

    let out = assay_bin().arg("modules").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("read-only mode active"), "stdout: {stdout}");
}

#[test]
fn cli_yaml_checks_honor_readonly() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_file(dir.path(), "check.lua", CLI_PROBE_SCRIPT);
    let config = write_file(
        dir.path(),
        "checks.yaml",
        &format!(
            "timeout: 30s\nretries: 0\nchecks:\n  - name: readonly-check\n    type: script\n    file: {}\n",
            script.display()
        ),
    );

    let out = assay_bin()
        .args(["run", "--readonly", config.to_str().unwrap()])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stderr={stderr}");
}
