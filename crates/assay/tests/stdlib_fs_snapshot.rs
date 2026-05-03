//! Lua tests for the assay.fs_snapshot stdlib module.
//!
//! Exercises btrfs / zfs / none backend dispatch by mocking shell.exec —
//! no real btrfs or zfs is required.

mod common;

use common::create_vm;

async fn run_fs_snapshot_lua(script_path: &str) {
    let script =
        std::fs::read_to_string(script_path).unwrap_or_else(|e| panic!("read {script_path}: {e}"));
    let vm = create_vm();
    vm.load(&script)
        .exec_async()
        .await
        .unwrap_or_else(|e| panic!("{script_path}: {e}"));
}

#[tokio::test]
async fn fs_snapshot_backend_dispatch() {
    run_fs_snapshot_lua("tests/fs_snapshot_lua/backend_dispatch.lua").await;
}
