//! Lua tests for the assay.nspawn stdlib module.
//!
//! These tests exercise the pure-data paths (config rendering, name
//! validation). Privileged operations (provision, destroy, lifecycle) need
//! a real systemd-machined and are exercised manually in the host's smoke
//! suite, not here.

mod common;

use common::create_vm;

async fn run_nspawn_lua(script_path: &str) {
    let script =
        std::fs::read_to_string(script_path).unwrap_or_else(|e| panic!("read {script_path}: {e}"));
    let vm = create_vm();
    vm.load(&script)
        .exec_async()
        .await
        .unwrap_or_else(|e| panic!("{script_path}: {e}"));
}

#[tokio::test]
async fn nspawn_config_render() {
    run_nspawn_lua("tests/nspawn_lua/config_render.lua").await;
}

#[tokio::test]
async fn nspawn_name_validation() {
    run_nspawn_lua("tests/nspawn_lua/name_validation.lua").await;
}
