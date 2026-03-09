mod common;

use common::run_lua;

#[tokio::test]
async fn test_disk_usage_root() {
    let script = r#"
        local usage = disk.usage("/")
        assert.not_nil(usage.total)
        assert.not_nil(usage.free)
        assert.not_nil(usage.used)
        assert.not_nil(usage.percent)
        assert.gt(usage.total, 0, "total should be > 0")
        assert.gt(usage.free, 0, "free should be > 0")
        assert.gt(usage.used, 0, "used should be > 0")
        assert.gt(usage.percent, 0, "percent should be > 0")
        assert.lt(usage.percent, 100.1, "percent should be <= 100")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_disk_usage_arithmetic() {
    let script = r#"
        local usage = disk.usage("/")
        -- used = total - free
        assert.eq(usage.used, usage.total - usage.free)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_disk_usage_invalid_path() {
    let result = run_lua(r#"disk.usage("/nonexistent/path/that/does/not/exist")"#).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_disk_sweep() {
    let script = r#"
        local dir = fs.tempdir()
        -- Create some files
        fs.write(dir .. "/old1.txt", "old content 1")
        fs.write(dir .. "/old2.txt", "old content 2")

        -- Sweep with age_secs=0 should remove everything (all files are > 0 seconds old)
        sleep(0.1)
        local removed = disk.sweep(dir, 0)
        assert.eq(removed, 2, "should have removed 2 files")

        -- Directory should now be empty
        local entries = fs.list(dir)
        local count = 0
        for _ in ipairs(entries) do count = count + 1 end
        assert.eq(count, 0, "directory should be empty after sweep")

        fs.remove(dir)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_disk_sweep_preserves_new_files() {
    let script = r#"
        local dir = fs.tempdir()
        fs.write(dir .. "/new.txt", "new content")

        -- Sweep with a large age should remove nothing
        local removed = disk.sweep(dir, 999999)
        assert.eq(removed, 0, "should not have removed any files")

        -- File should still exist
        assert.eq(fs.exists(dir .. "/new.txt"), true)

        fs.remove(dir)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_disk_dir_size() {
    let script = r#"
        local dir = fs.tempdir()
        fs.write(dir .. "/a.txt", "hello")
        fs.write(dir .. "/b.txt", "world!")

        local size = disk.dir_size(dir)
        -- "hello" = 5 bytes, "world!" = 6 bytes
        assert.eq(size, 11, "expected 11 bytes total")

        fs.remove(dir)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_disk_dir_size_empty() {
    let script = r#"
        local dir = fs.tempdir()
        local size = disk.dir_size(dir)
        assert.eq(size, 0, "empty dir should be 0 bytes")
        fs.remove(dir)
    "#;
    run_lua(script).await.unwrap();
}
