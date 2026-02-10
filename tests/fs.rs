mod common;

use common::run_lua;

#[tokio::test]
async fn test_fs_read() {
    let script = r#"
        local content = fs.read("Cargo.toml")
        assert.contains(content, "assay")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_fs_read_nonexistent() {
    let result = run_lua(r#"fs.read("/nonexistent/file.txt")"#).await;
    assert!(result.is_err());
}
