mod common;

use common::run_lua;

// ──────────────────────── string.split ────────────────────────

#[tokio::test]
async fn test_string_split_default_whitespace() {
    let script = r#"
        local parts = string.split("alpha  beta\tgamma\n")
        assert.eq(#parts, 3)
        assert.eq(parts[1], "alpha")
        assert.eq(parts[2], "beta")
        assert.eq(parts[3], "gamma")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_string_split_leading_trailing_whitespace() {
    let script = r#"
        local parts = string.split("   a b c   ")
        assert.eq(#parts, 3)
        assert.eq(parts[1], "a")
        assert.eq(parts[3], "c")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_string_split_custom_separator() {
    let script = r#"
        local parts = string.split("a,b,c,d", ",")
        assert.eq(#parts, 4)
        assert.eq(parts[1], "a")
        assert.eq(parts[4], "d")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_string_split_literal_sep_not_pattern() {
    // '.' is a Lua pattern "any char" — but string.split treats
    // the separator as a literal string, not a pattern.
    let script = r#"
        local parts = string.split("a.b.c", ".")
        assert.eq(#parts, 3)
        assert.eq(parts[1], "a")
        assert.eq(parts[2], "b")
        assert.eq(parts[3], "c")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_string_split_multichar_separator() {
    let script = r#"
        local parts = string.split("foo::bar::baz", "::")
        assert.eq(#parts, 3)
        assert.eq(parts[1], "foo")
        assert.eq(parts[2], "bar")
        assert.eq(parts[3], "baz")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_string_split_single_element() {
    let script = r#"
        local parts = string.split("lonely")
        assert.eq(#parts, 1)
        assert.eq(parts[1], "lonely")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_string_split_empty_string_whitespace() {
    let script = r#"
        local parts = string.split("")
        assert.eq(#parts, 0, "empty string yields empty array under whitespace split")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_string_split_empty_string_custom_sep() {
    // With a literal separator, an empty string still counts as a
    // single empty field (matches Rust str::split / Python str.split).
    let script = r#"
        local parts = string.split("", ",")
        assert.eq(#parts, 1)
        assert.eq(parts[1], "")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_string_split_trailing_separator() {
    // "a,b," splits into {"a", "b", ""}.
    let script = r#"
        local parts = string.split("a,b,", ",")
        assert.eq(#parts, 3)
        assert.eq(parts[1], "a")
        assert.eq(parts[2], "b")
        assert.eq(parts[3], "")
    "#;
    run_lua(script).await.unwrap();
}
