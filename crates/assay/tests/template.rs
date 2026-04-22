mod common;

use common::run_lua;

#[tokio::test]
async fn test_template_render_string_simple() {
    run_lua(
        r#"
        local result = template.render_string("Hello {{ name }}!", {name = "World"})
        assert.eq(result, "Hello World!")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_template_render_string_loop() {
    run_lua(
        r#"
        local tmpl = "{% for item in items %}{{ item }},{% endfor %}"
        local result = template.render_string(tmpl, {items = {"a", "b", "c"}})
        assert.eq(result, "a,b,c,")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_template_render_string_conditional() {
    run_lua(
        r#"
        local tmpl = "{% if active %}yes{% else %}no{% endif %}"
        local r1 = template.render_string(tmpl, {active = true})
        assert.eq(r1, "yes")
        local r2 = template.render_string(tmpl, {active = false})
        assert.eq(r2, "no")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_template_render_string_nested_objects() {
    run_lua(
        r#"
        local tmpl = "{{ user.name }} is {{ user.age }}"
        local result = template.render_string(tmpl, {user = {name = "Alice", age = 30}})
        assert.eq(result, "Alice is 30")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_template_render_string_filters() {
    run_lua(
        r#"
        local tmpl = "{{ name | upper }} has {{ items | length }} items"
        local result = template.render_string(tmpl, {name = "alice", items = {1, 2, 3}})
        assert.eq(result, "ALICE has 3 items")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_template_render_from_file() {
    let dir = std::env::temp_dir().join("assay_test_template_render");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("page.html");
    std::fs::write(&path, "<h1>{{ title }}</h1>").unwrap();

    let script = format!(
        r#"
        local result = template.render("{path}", {{title = "Home"}})
        assert.eq(result, "<h1>Home</h1>")
        "#,
        path = path.display().to_string().replace('\\', "\\\\")
    );
    run_lua(&script).await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_template_render_string_no_vars() {
    run_lua(
        r#"
        local result = template.render_string("static content", {})
        assert.eq(result, "static content")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_template_render_string_nil_vars() {
    run_lua(
        r#"
        local result = template.render_string("static content", nil)
        assert.eq(result, "static content")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_template_render_string_undefined_variable() {
    run_lua(
        r#"
        local result = template.render_string("Hello {{ name }}!", {})
        assert.eq(result, "Hello !")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_template_render_string_invalid_syntax() {
    let result = run_lua(
        r#"
        template.render_string("{% invalid %}", {})
    "#,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("template.render_string"),
        "error should mention template.render_string: {err}"
    );
}

#[tokio::test]
async fn test_template_render_file_not_found() {
    let result = run_lua(
        r#"
        template.render("/nonexistent/template.html", {})
    "#,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("template.render"),
        "error should mention template.render: {err}"
    );
}

#[tokio::test]
async fn test_template_render_string_loop_with_objects() {
    run_lua(
        r#"
        local tmpl = "{% for item in items %}{{ item.name }}:{{ item.price }};{% endfor %}"
        local result = template.render_string(tmpl, {
            items = {{name = "A", price = 10}, {name = "B", price = 20}}
        })
        assert.eq(result, "A:10;B:20;")
    "#,
    )
    .await
    .unwrap();
}
