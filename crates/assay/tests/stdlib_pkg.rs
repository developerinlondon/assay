mod common;

use common::create_vm;

async fn run_pkg_lua(script_path: &str) {
    let script = std::fs::read_to_string(script_path)
        .unwrap_or_else(|e| panic!("read {script_path}: {e}"));
    let vm = create_vm();
    vm.load(&script)
        .exec_async()
        .await
        .unwrap_or_else(|e| panic!("{script_path}: {e}"));
}

#[tokio::test]
async fn pkg_catalog_validation() {
    run_pkg_lua("tests/pkg_lua/catalog_validation.lua").await;
}
