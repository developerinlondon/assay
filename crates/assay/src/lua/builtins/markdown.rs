/// Markdown to HTML conversion builtin.
///
/// Usage from Lua:
///   markdown.to_html("# Hello")           → "<h1>Hello</h1>\n"
///   markdown.to_html(fs.read("README.md")) → full HTML string
pub fn register_markdown(lua: &mlua::Lua) -> mlua::Result<()> {
    use pulldown_cmark::{Options, Parser, html};

    let md = lua.create_table()?;

    // markdown.to_html(source) → html_string
    md.set(
        "to_html",
        lua.create_function(|_, source: String| {
            let mut opts = Options::empty();
            opts.insert(Options::ENABLE_TABLES);
            opts.insert(Options::ENABLE_STRIKETHROUGH);
            opts.insert(Options::ENABLE_TASKLISTS);

            let parser = Parser::new_ext(&source, opts);
            let mut html_output = String::new();
            html::push_html(&mut html_output, parser);
            Ok(html_output)
        })?,
    )?;

    lua.globals().set("markdown", md)?;
    Ok(())
}
