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

#[tokio::test]
async fn test_fs_write_basic() {
    let dir = std::env::temp_dir().join("assay_test_write_basic");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.txt");

    let script = format!(
        r#"fs.write("{}", "hello from assay")"#,
        path.display().to_string().replace('\\', "\\\\")
    );
    run_lua(&script).await.unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "hello from assay");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_fs_write_creates_parent_dirs() {
    let dir = std::env::temp_dir().join("assay_test_write_nested/a/b/c");
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("assay_test_write_nested"));
    let path = dir.join("deep.txt");

    let script = format!(
        r#"fs.write("{}", "nested content")"#,
        path.display().to_string().replace('\\', "\\\\")
    );
    run_lua(&script).await.unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "nested content");
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("assay_test_write_nested"));
}

#[tokio::test]
async fn test_fs_write_overwrite() {
    let dir = std::env::temp_dir().join("assay_test_write_overwrite");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("overwrite.txt");

    std::fs::write(&path, "original").unwrap();

    let script = format!(
        r#"fs.write("{}", "replaced")"#,
        path.display().to_string().replace('\\', "\\\\")
    );
    run_lua(&script).await.unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "replaced");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_fs_write_then_read() {
    let dir = std::env::temp_dir().join("assay_test_write_read");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("roundtrip.txt");

    let script = format!(
        r#"
        fs.write("{path}", "roundtrip data")
        local content = fs.read("{path}")
        assert.eq(content, "roundtrip data")
        "#,
        path = path.display().to_string().replace('\\', "\\\\")
    );
    run_lua(&script).await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_fs_write_invalid_path() {
    let result = run_lua(r#"fs.write("/proc/0/nonexistent", "data")"#).await;
    assert!(result.is_err());
}
