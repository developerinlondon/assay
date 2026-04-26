mod common;

use common::run_lua;

#[tokio::test]
async fn test_require_version() {
    let script = r#"
        local v = require("assay.version")
        assert.not_nil(v)
        assert.not_nil(v.compare)
        assert.not_nil(v.max)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_semver_basics() {
    let script = r#"
        local v = require("assay.version")
        assert.eq(v.compare("1.2.3", "1.2.4"), -1)
        assert.eq(v.compare("1.2.4", "1.2.3", "semver"), 1)
        assert.eq(v.compare("1.2.3", "1.2.3", "semver"), 0)
        assert.eq(v.compare("v0.13.1", "0.13.2", "semver"), -1)
        assert.eq(v.compare("v1.2.3", "1.2.3", "semver"), 0)
        -- default scheme is semver
        assert.eq(v.compare("1.10.0", "1.9.0"), 1)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_semver_prerelease_ordering() {
    let script = r#"
        local v = require("assay.version")
        -- prerelease < release
        assert.eq(v.compare("1.0.0-alpha", "1.0.0", "semver"), -1)
        assert.eq(v.compare("1.0.0", "1.0.0-alpha", "semver"), 1)
        -- alpha < alpha.1
        assert.eq(v.compare("1.0.0-alpha", "1.0.0-alpha.1", "semver"), -1)
        -- alpha.1 < beta
        assert.eq(v.compare("1.0.0-alpha.1", "1.0.0-beta", "semver"), -1)
        -- beta < rc.1
        assert.eq(v.compare("1.0.0-beta", "1.0.0-rc.1", "semver"), -1)
        -- rc.1 < final
        assert.eq(v.compare("1.0.0-rc.1", "1.0.0", "semver"), -1)
        -- numeric prerelease id < alphanumeric
        assert.eq(v.compare("1.0.0-1", "1.0.0-alpha", "semver"), -1)
        -- build metadata ignored
        assert.eq(v.compare("1.0.0+build1", "1.0.0+build2", "semver"), 0)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_debian_epoch() {
    let script = r#"
        local v = require("assay.version")
        assert.eq(v.compare("1:1.0", "2.0", "debian"), 1)
        assert.eq(v.compare("2.0", "1:1.0", "debian"), -1)
        assert.eq(v.compare("1:1.0", "1:1.0", "debian"), 0)
        assert.eq(v.compare("1:1.84.3-noble1", "1.84.2", "debian"), 1)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_debian_tilde() {
    let script = r#"
        local v = require("assay.version")
        assert.eq(v.compare("1.0~rc1", "1.0", "debian"), -1)
        assert.eq(v.compare("1.0", "1.0~rc1", "debian"), 1)
        assert.eq(v.compare("1.0~rc1", "1.0~rc2", "debian"), -1)
        assert.eq(v.compare("1.0~rc2", "1.0~rc1", "debian"), 1)
        -- tilde sorts before letters too
        assert.eq(v.compare("1.0~", "1.0a", "debian"), -1)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_debian_revision() {
    let script = r#"
        local v = require("assay.version")
        assert.eq(v.compare("1.0-1", "1.0-2", "debian"), -1)
        assert.eq(v.compare("1.0-2", "1.0-1", "debian"), 1)
        assert.eq(v.compare("1.0-1", "1.0-1", "debian"), 0)
        assert.eq(v.compare("1.0-10", "1.0-2", "debian"), 1)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_rpm_no_tilde_rule() {
    let script = r#"
        local v = require("assay.version")
        -- non-tilde cases match debian
        assert.eq(v.compare("1.2.3", "1.2.4", "rpm"), -1)
        assert.eq(v.compare("1.0-1", "1.0-2", "rpm"), -1)
        assert.eq(v.compare("1.10", "1.9", "rpm"), 1)
        -- in rpm, tilde is treated as a normal character so 1.0~rc1 > 1.0
        -- (since '~' has byte value 126, after digit/letter handling)
        local c = v.compare("1.0~rc1", "1.0", "rpm")
        assert.eq(c, 1)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_numeric() {
    let script = r#"
        local v = require("assay.version")
        assert.eq(v.compare("1.10", "1.9", "numeric"), 1)
        assert.eq(v.compare("1.9", "1.10", "numeric"), -1)
        assert.eq(v.compare("1.2", "1.2.0", "numeric"), 0)
        assert.eq(v.compare("1.2.0", "1.2", "numeric"), 0)
        assert.eq(v.compare("2.0.0", "1.99.99", "numeric"), 1)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_max() {
    let script = r#"
        local v = require("assay.version")
        assert.eq(v.max({ "1.2", "1.10", "1.9" }, "semver"), "1.10")
        assert.eq(v.max({ "1.0.0-alpha", "1.0.0-beta", "1.0.0-rc.1", "1.0.0" }, "semver"), "1.0.0")
        assert.eq(v.max({ "1.0~rc1", "1.0~rc2", "1.0" }, "debian"), "1.0")
        assert.eq(v.max({ "1.10", "1.9", "1.2" }, "numeric"), "1.10")
        -- default scheme is semver
        assert.eq(v.max({ "1.2", "1.10", "1.9" }), "1.10")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_unknown_scheme_errors() {
    let script = r#"
        local v = require("assay.version")
        local ok, err = pcall(function() return v.compare("1.0", "1.1", "bogus") end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "unknown scheme")
    "#;
    run_lua(script).await.unwrap();
}
