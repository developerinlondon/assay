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

#[tokio::test]
async fn test_fs_copy() {
    let script = r#"
        local dir = fs.tempdir()
        fs.write(dir .. "/source.txt", "copy me")
        local bytes = fs.copy(dir .. "/source.txt", dir .. "/dest.txt")
        assert.eq(bytes, 7)
        local content = fs.read(dir .. "/dest.txt")
        assert.eq(content, "copy me")
        -- Source should still exist
        assert.eq(fs.exists(dir .. "/source.txt"), true)
        fs.remove(dir)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_fs_copy_nonexistent() {
    let result = run_lua(r#"fs.copy("/nonexistent/src", "/tmp/dst")"#).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_fs_rename() {
    let script = r#"
        local dir = fs.tempdir()
        fs.write(dir .. "/before.txt", "rename me")
        fs.rename(dir .. "/before.txt", dir .. "/after.txt")
        assert.eq(fs.exists(dir .. "/before.txt"), false)
        assert.eq(fs.exists(dir .. "/after.txt"), true)
        local content = fs.read(dir .. "/after.txt")
        assert.eq(content, "rename me")
        fs.remove(dir)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_fs_rename_nonexistent() {
    let result = run_lua(r#"fs.rename("/nonexistent/src", "/tmp/dst")"#).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_fs_glob() {
    let script = r#"
        local dir = fs.tempdir()
        fs.write(dir .. "/a.txt", "a")
        fs.write(dir .. "/b.txt", "b")
        fs.write(dir .. "/c.log", "c")

        local matches = fs.glob(dir .. "/*.txt")
        local count = 0
        for _ in ipairs(matches) do count = count + 1 end
        assert.eq(count, 2, "should match 2 .txt files")

        local all = fs.glob(dir .. "/*")
        local all_count = 0
        for _ in ipairs(all) do all_count = all_count + 1 end
        assert.eq(all_count, 3, "should match all 3 files")

        fs.remove(dir)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_fs_glob_recursive() {
    let script = r#"
        local dir = fs.tempdir()
        fs.mkdir(dir .. "/sub")
        fs.write(dir .. "/top.txt", "top")
        fs.write(dir .. "/sub/nested.txt", "nested")

        local matches = fs.glob(dir .. "/**/*.txt")
        local count = 0
        for _ in ipairs(matches) do count = count + 1 end
        assert.eq(count, 2, "should match 2 .txt files recursively")

        fs.remove(dir)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_fs_glob_no_match() {
    let script = r#"
        local matches = fs.glob("/tmp/assay-nonexistent-pattern-*.xyz")
        local count = 0
        for _ in ipairs(matches) do count = count + 1 end
        assert.eq(count, 0)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_fs_glob_invalid_pattern() {
    let result = run_lua(r#"fs.glob("[invalid")"#).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_fs_tempdir() {
    let script = r#"
        local dir = fs.tempdir()
        assert.not_nil(dir)
        assert.gt(#dir, 0, "tempdir path should not be empty")
        assert.eq(fs.exists(dir), true)

        -- Should be able to write into it
        fs.write(dir .. "/test.txt", "hello")
        assert.eq(fs.read(dir .. "/test.txt"), "hello")

        fs.remove(dir)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_fs_chmod() {
    let script = r#"
        local dir = fs.tempdir()
        local path = dir .. "/chmod_test.txt"
        fs.write(path, "test")

        -- Set to 0o644 = 420 decimal
        fs.chmod(path, 420)
        local stat = fs.stat(path)
        assert.eq(stat.is_file, true)

        -- Set to 0o755 = 493 decimal
        fs.chmod(path, 493)

        fs.remove(dir)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_fs_chmod_nonexistent() {
    let result = run_lua(r#"fs.chmod("/nonexistent/file", 420)"#).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_fs_readdir() {
    let script = r#"
        local dir = fs.tempdir()
        fs.write(dir .. "/a.txt", "a")
        fs.mkdir(dir .. "/sub")
        fs.write(dir .. "/sub/b.txt", "b")

        local entries = fs.readdir(dir)
        local count = 0
        local has_file = false
        local has_dir = false
        local has_nested = false
        for _, e in ipairs(entries) do
            count = count + 1
            if e.path == "a.txt" and e.type == "file" then has_file = true end
            if e.path == "sub" and e.type == "directory" then has_dir = true end
            if e.path == "sub/b.txt" and e.type == "file" then has_nested = true end
        end
        assert.eq(count, 3, "should have 3 entries (a.txt, sub, sub/b.txt)")
        assert.eq(has_file, true, "should have a.txt")
        assert.eq(has_dir, true, "should have sub directory")
        assert.eq(has_nested, true, "should have sub/b.txt")

        fs.remove(dir)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_fs_readdir_with_depth() {
    let script = r#"
        local dir = fs.tempdir()
        fs.write(dir .. "/top.txt", "top")
        fs.mkdir(dir .. "/a")
        fs.write(dir .. "/a/mid.txt", "mid")
        fs.mkdir(dir .. "/a/b")
        fs.write(dir .. "/a/b/deep.txt", "deep")

        -- depth=1 should only list top level
        local entries = fs.readdir(dir, {depth = 1})
        local count = 0
        local has_deep = false
        for _, e in ipairs(entries) do
            count = count + 1
            if e.path == "a/b/deep.txt" then has_deep = true end
        end
        -- Should have: top.txt, a, a/mid.txt (a is listed, mid.txt at depth 1 inside a)
        -- but NOT a/b/deep.txt (that's depth 2)
        assert.eq(has_deep, false, "should not include deep entries at depth=1")

        fs.remove(dir)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_fs_readdir_nonexistent() {
    let result = run_lua(r#"fs.readdir("/nonexistent/path")"#).await;
    assert!(result.is_err());
}
