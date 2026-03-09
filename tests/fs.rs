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

#[tokio::test]
async fn test_fs_exists() {
    let script = r#"
        assert.eq(fs.exists("Cargo.toml"), true)
        assert.eq(fs.exists("/nonexistent/path"), false)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_fs_mkdir_and_remove() {
    let dir = std::env::temp_dir().join("assay_test_mkdir");
    let _ = std::fs::remove_dir_all(&dir);

    let script = format!(
        r#"
        fs.mkdir("{}")
        assert.eq(fs.exists("{}"), true)
        fs.remove("{}")
        assert.eq(fs.exists("{}"), false)
        "#,
        dir.display(), dir.display(), dir.display(), dir.display()
    );
    run_lua(&script).await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_fs_stat() {
    let dir = std::env::temp_dir().join("assay_test_stat");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("stat_test.txt");
    std::fs::write(&path, "hello stat").unwrap();

    let script = format!(
        r#"
        local info = fs.stat("{}")
        assert.eq(info.is_file, true)
        assert.eq(info.is_dir, false)
        assert.eq(info.size, 10)
        assert.not_nil(info.modified)
        "#,
        path.display().to_string().replace('\\', "\\\\")
    );
    run_lua(&script).await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_fs_stat_directory() {
    let script = r#"
        local info = fs.stat("src")
        assert.eq(info.is_dir, true)
        assert.eq(info.is_file, false)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_fs_stat_nonexistent() {
    let result = run_lua(r#"fs.stat("/nonexistent/file")"#).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_fs_list() {
    let dir = std::env::temp_dir().join("assay_test_list");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    std::fs::write(dir.join("b.txt"), "b").unwrap();
    std::fs::create_dir_all(dir.join("subdir")).unwrap();

    let script = format!(
        r#"
        local entries = fs.list("{}")
        assert.not_nil(entries)
        local count = 0
        local has_file = false
        local has_dir = false
        for _, e in ipairs(entries) do
            count = count + 1
            if e.type == "file" then has_file = true end
            if e.type == "directory" then has_dir = true end
        end
        assert.eq(count, 3)
        assert.eq(has_file, true)
        assert.eq(has_dir, true)
        "#,
        dir.display().to_string().replace('\\', "\\\\")
    );
    run_lua(&script).await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_fs_remove_file() {
    let dir = std::env::temp_dir().join("assay_test_remove_file");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("removeme.txt");
    std::fs::write(&path, "bye").unwrap();

    let script = format!(
        r#"
        assert.eq(fs.exists("{}"), true)
        fs.remove("{}")
        assert.eq(fs.exists("{}"), false)
        "#,
        path.display().to_string().replace('\\', "\\\\"),
        path.display().to_string().replace('\\', "\\\\"),
        path.display().to_string().replace('\\', "\\\\")
    );
    run_lua(&script).await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_fs_remove_nonexistent() {
    let result = run_lua(r#"fs.remove("/nonexistent/file.txt")"#).await;
    assert!(result.is_err());
}
