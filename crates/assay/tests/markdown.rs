mod common;

use common::eval_lua;

#[tokio::test]
async fn test_markdown_to_html_heading() {
    let result: String = eval_lua(
        r##"return markdown.to_html("# Hello")"##,
    )
    .await;
    assert!(
        result.contains("<h1>Hello</h1>"),
        "expected <h1>Hello</h1> in: {result}"
    );
}

#[tokio::test]
async fn test_markdown_to_html_paragraph() {
    let result: String = eval_lua(
        r#"return markdown.to_html("Hello **world**")"#,
    )
    .await;
    assert!(
        result.contains("<strong>world</strong>"),
        "expected <strong>world</strong> in: {result}"
    );
}

#[tokio::test]
async fn test_markdown_to_html_table() {
    let result: String = eval_lua(
        r#"return markdown.to_html("| A | B |\n|---|---|\n| 1 | 2 |")"#,
    )
    .await;
    assert!(
        result.contains("<table>"),
        "expected <table> in: {result}"
    );
}

#[tokio::test]
async fn test_markdown_to_html_code_block() {
    let result: String = eval_lua(
        r#"return markdown.to_html("```lua\nprint('hi')\n```")"#,
    )
    .await;
    assert!(
        result.contains("<code"),
        "expected <code in: {result}"
    );
}

#[tokio::test]
async fn test_markdown_to_html_list() {
    let result: String = eval_lua(
        r#"return markdown.to_html("- one\n- two\n- three")"#,
    )
    .await;
    assert!(
        result.contains("<ul>") && result.contains("<li>"),
        "expected <ul> and <li> in: {result}"
    );
}

#[tokio::test]
async fn test_markdown_is_global() {
    let result: bool = eval_lua(
        r#"return type(markdown) == "table" and type(markdown.to_html) == "function""#,
    )
    .await;
    assert!(result);
}
