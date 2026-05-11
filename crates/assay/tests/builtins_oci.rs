mod common;

use common::run_lua;

#[tokio::test]
async fn test_oci_builtin_available() {
    let script = r#"
        assert.not_nil(oci)
        assert.not_nil(oci.copy)
        assert.not_nil(oci.tag)
        assert.not_nil(oci.mutate)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_oci_copy_missing_auth_uses_anonymous() {
    let script = r#"
        -- Should fail with a registry error, not a Lua error
        local ok, err = pcall(function()
            oci.copy("nonexistent.registry/foo:latest", "other.registry/bar:latest")
        end)
        assert.eq(ok, false)
        -- Error should be about pull/push failure, not Lua type error
        assert.not_nil(string.find(err, "pull failed") or string.find(err, "no such host"))
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_oci_tag_typechecks() {
    let script = r#"
        -- Missing args should error
        local ok, _ = pcall(function() oci.tag() end)
        assert.eq(ok, false)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_oci_mutate_typechecks() {
    let script = r#"
        -- mutate needs files table
        local ok, _ = pcall(function()
            oci.mutate("src", "dst", "not a table")
        end)
        assert.eq(ok, false)
    "#;
    run_lua(script).await.unwrap();
}
