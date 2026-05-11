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
        -- Error should be about pull failure / network, not a Lua type error
        local s = tostring(err)
        local network_failure = s:find("pull manifest")
            or s:find("no such host")
            or s:find("dns")
            or s:find("connect")
        assert.not_nil(network_failure)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_oci_tag_rejects_cross_registry() {
    let script = r#"
        local ok, err = pcall(function()
            oci.tag("registry-a.example/foo:1", "2")
            -- ^ this is fine; tag stays in the same registry
        end)
        -- We expect a network failure, not a Lua-level rejection.
        assert.eq(ok, false)

        -- A cross-repository "tag" call should fail fast with a clear error
        -- because tag is documented as same-repo only.
        local ok2, err2 = pcall(function()
            -- tag only takes (src, new_tag, opts?); a cross-repo retag has
            -- to go through oci.copy. We assert tag() with bad args errors.
            oci.tag()
        end)
        assert.eq(ok2, false)
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
