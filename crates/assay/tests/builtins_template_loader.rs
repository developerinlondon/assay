mod common;

use common::run_lua;
use std::fs;
use tempfile::tempdir;

#[tokio::test]
async fn render_with_loader_supports_extends_and_include() {
    let dir = tempdir().unwrap();
    let templates = dir.path().join("templates");
    fs::create_dir(&templates).unwrap();
    fs::write(
        templates.join("base.html"),
        "<html>{% block content %}{% endblock %}</html>",
    )
    .unwrap();
    fs::write(
        templates.join("page.html"),
        "{% extends \"base.html\" %}{% block content %}<ul>{% include \"row.html\" %}</ul>{% endblock %}",
    )
    .unwrap();
    fs::write(templates.join("row.html"), "<li>{{ name }}</li>").unwrap();

    let script = format!(
        r#"
        local out = template.render_with_loader({:?}, "page.html", {{ name = "hi" }})
        assert.eq(out, "<html><ul><li>hi</li></ul></html>")
        "#,
        templates.to_string_lossy()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn render_with_loader_supports_import_macros() {
    let dir = tempdir().unwrap();
    let templates = dir.path().join("templates");
    fs::create_dir(&templates).unwrap();
    fs::write(
        templates.join("macros.html"),
        "{% macro greet(who) %}hi {{ who }}{% endmacro %}",
    )
    .unwrap();
    fs::write(
        templates.join("page.html"),
        "{% import \"macros.html\" as m %}{{ m.greet(name) }}",
    )
    .unwrap();

    let script = format!(
        r#"
        local out = template.render_with_loader({:?}, "page.html", {{ name = "world" }})
        assert.eq(out, "hi world")
        "#,
        templates.to_string_lossy()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn render_with_loader_errors_on_missing_template() {
    let dir = tempdir().unwrap();
    let templates = dir.path().join("templates");
    fs::create_dir(&templates).unwrap();

    let script = format!(
        r#"
        local ok, err = pcall(template.render_with_loader, {:?}, "nope.html", {{}})
        assert.eq(ok, false)
        assert.eq(string.find(tostring(err), "render_with_loader") ~= nil, true)
        "#,
        templates.to_string_lossy()
    );
    run_lua(&script).await.unwrap();
}
