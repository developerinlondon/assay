mod common;

use common::run_lua_with_lib_path;

fn setup_lib_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("assay_test_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn test_fs_require_loads_external_module() {
    let dir = setup_lib_dir("fs_require_basic");
    std::fs::write(
        dir.join("mylib.lua"),
        "local M = {}\nfunction M.hello() return \"world\" end\nreturn M\n",
    )
    .unwrap();

    let script = r#"
        local mylib = require("assay.mylib")
        assert.eq(mylib.hello(), "world")
    "#;
    let result = run_lua_with_lib_path(script, dir.to_str().unwrap()).await;
    cleanup(&dir);
    result.unwrap();
}

#[tokio::test]
async fn test_fs_require_embedded_takes_priority() {
    let dir = setup_lib_dir("fs_require_priority");
    std::fs::write(
        dir.join("vault.lua"),
        "error('should not load filesystem vault')\n",
    )
    .unwrap();

    let script = r#"
        local vault = require("assay.vault")
        assert.not_nil(vault)
        assert.not_nil(vault.client)
    "#;
    let result = run_lua_with_lib_path(script, dir.to_str().unwrap()).await;
    cleanup(&dir);
    result.unwrap();
}

#[tokio::test]
async fn test_fs_require_nonexistent_module_errors() {
    let dir = setup_lib_dir("fs_require_missing");

    let script = r#"
        local missing = require("assay.does_not_exist")
    "#;
    let result = run_lua_with_lib_path(script, dir.to_str().unwrap()).await;
    cleanup(&dir);
    assert!(result.is_err(), "requiring nonexistent module should error");
}

#[tokio::test]
async fn test_fs_require_module_has_access_to_builtins() {
    let dir = setup_lib_dir("fs_require_builtins");
    std::fs::write(
        dir.join("checker.lua"),
        r#"
local M = {}
function M.check_url(url)
    local resp = http.get(url)
    return resp.status
end
return M
"#,
    )
    .unwrap();

    let script = r#"
        local checker = require("assay.checker")
        assert.not_nil(checker)
        assert.not_nil(checker.check_url)
    "#;
    let result = run_lua_with_lib_path(script, dir.to_str().unwrap()).await;
    cleanup(&dir);
    result.unwrap();
}

#[tokio::test]
async fn test_fs_require_module_can_require_embedded() {
    let dir = setup_lib_dir("fs_require_chain");
    std::fs::write(
        dir.join("wrapper.lua"),
        r#"
local vault = require("assay.vault")
local M = {}
function M.has_vault()
    return vault ~= nil and vault.client ~= nil
end
return M
"#,
    )
    .unwrap();

    let script = r#"
        local wrapper = require("assay.wrapper")
        assert.eq(wrapper.has_vault(), true)
    "#;
    let result = run_lua_with_lib_path(script, dir.to_str().unwrap()).await;
    cleanup(&dir);
    result.unwrap();
}

#[tokio::test]
async fn test_fs_require_default_path_without_env() {
    let script = r#"
        local ok, err = pcall(require, "assay.nonexistent_lib_xyz")
        assert.eq(ok, false)
        assert.contains(tostring(err), "nonexistent_lib_xyz")
    "#;
    // Uses default /libs path (no explicit lib path override)
    common::run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_fs_require_caches_module() {
    let dir = setup_lib_dir("fs_require_cache");
    std::fs::write(
        dir.join("counter.lua"),
        "local M = { count = 0 }\nM.count = M.count + 1\nreturn M\n",
    )
    .unwrap();

    let script = r#"
        local a = require("assay.counter")
        local b = require("assay.counter")
        a.marker = "cached"
        assert.eq(b.marker, "cached", "require should return cached module (same table)")
        assert.eq(a.count, 1, "module should only execute once")
    "#;
    let result = run_lua_with_lib_path(script, dir.to_str().unwrap()).await;
    cleanup(&dir);
    result.unwrap();
}

#[tokio::test]
async fn test_fs_require_non_assay_prefix_ignored() {
    let script = r#"
        local ok, err = pcall(require, "mycompany.mylib")
        assert.eq(ok, false)
    "#;
    common::run_lua(script).await.unwrap();
}
