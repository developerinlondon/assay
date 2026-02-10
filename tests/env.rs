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
