mod common;

use common::eval_lua;

#[tokio::test]
async fn test_env_get_existing() {
    unsafe { std::env::set_var("ASSAY_TEST_VAR", "hello") };
    let result: String = eval_lua(r#"return env.get("ASSAY_TEST_VAR")"#).await;
    assert_eq!(result, "hello");
    unsafe { std::env::remove_var("ASSAY_TEST_VAR") };
}

#[tokio::test]
async fn test_env_get_missing() {
    let script = r#"
        local val = env.get("ASSAY_NONEXISTENT_VAR_12345")
        assert.eq(val, nil)
    "#;
    common::run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_env_set() {
    let script = r#"
        -- Set a new env var
        env.set("ASSAY_TEST_SET_VAR", "from_lua")
        local val = env.get("ASSAY_TEST_SET_VAR")
        assert.eq(val, "from_lua")

        -- Unset it by passing nil
        env.set("ASSAY_TEST_SET_VAR", nil)
        local val2 = env.get("ASSAY_TEST_SET_VAR")
        assert.eq(val2, nil)
    "#;
    common::run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_env_set_overwrite() {
    let script = r#"
        env.set("ASSAY_TEST_OVERWRITE", "first")
        assert.eq(env.get("ASSAY_TEST_OVERWRITE"), "first")
        env.set("ASSAY_TEST_OVERWRITE", "second")
        assert.eq(env.get("ASSAY_TEST_OVERWRITE"), "second")
        env.set("ASSAY_TEST_OVERWRITE", nil)
    "#;
    common::run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_env_list() {
    let script = r#"
        env.set("ASSAY_TEST_LIST_VAR", "present")
        local vars = env.list()
        assert.not_nil(vars)

        local count = 0
        local found = false
        for _, v in ipairs(vars) do
            count = count + 1
            assert.not_nil(v.key)
            assert.not_nil(v.value)
            if v.key == "ASSAY_TEST_LIST_VAR" and v.value == "present" then
                found = true
            end
        end
        assert.gt(count, 0, "should have at least one env var")
        assert.eq(found, true, "should find ASSAY_TEST_LIST_VAR in list")

        env.set("ASSAY_TEST_LIST_VAR", nil)
    "#;
    common::run_lua(script).await.unwrap();
}
