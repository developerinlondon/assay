mod common;

use common::run_lua;

#[tokio::test]
async fn test_os_hostname() {
    let script = r#"
        local h = os.hostname()
        assert.not_nil(h)
        -- hostname should be a non-empty string
        assert.gt(#h, 0, "hostname should not be empty")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_os_arch() {
    let script = r#"
        local arch = os.arch()
        assert.not_nil(arch)
        assert.gt(#arch, 0, "arch should not be empty")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_os_arch_known_value() {
    // On this machine it should be x86_64 or aarch64
    let script = r#"
        local arch = os.arch()
        local known = (arch == "x86_64" or arch == "aarch64" or arch == "arm")
        assert.eq(known, true, "arch should be a known architecture, got: " .. arch)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_os_platform() {
    let script = r#"
        local p = os.platform()
        assert.not_nil(p)
        assert.gt(#p, 0, "platform should not be empty")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_os_platform_known_value() {
    let script = r#"
        local p = os.platform()
        local known = (p == "linux" or p == "macos" or p == "windows")
        assert.eq(known, true, "platform should be a known OS, got: " .. p)
    "#;
    run_lua(script).await.unwrap();
}
