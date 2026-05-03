//! Lua tests for the assay.rustic stdlib module.
//!
//! These exercise command + env construction by mocking shell.exec —
//! no real rustic CLI is invoked, so the tests run anywhere.

mod common;

use common::create_vm;

async fn run_rustic_lua(script_path: &str) {
    let script =
        std::fs::read_to_string(script_path).unwrap_or_else(|e| panic!("read {script_path}: {e}"));
    let vm = create_vm();
    vm.load(&script)
        .exec_async()
        .await
        .unwrap_or_else(|e| panic!("{script_path}: {e}"));
}

#[tokio::test]
async fn rustic_command_construction() {
    run_rustic_lua("tests/rustic_lua/command_construction.lua").await;
}
