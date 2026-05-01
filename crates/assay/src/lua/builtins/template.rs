use super::json::lua_value_to_json;
use mlua::{Lua, Value};

pub fn register_template(lua: &Lua) -> mlua::Result<()> {
    let tmpl_table = lua.create_table()?;

    let render_string_fn = lua.create_function(|_, (template_str, vars): (String, Value)| {
        let json_vars = match &vars {
            Value::Table(_) => lua_value_to_json(&vars)?,
            Value::Nil => serde_json::Value::Object(serde_json::Map::new()),
            _ => {
                return Err(mlua::Error::runtime(
                    "template.render_string: second argument must be a table or nil",
                ));
            }
        };
        let mini_vars = minijinja::value::Value::from_serialize(&json_vars);
        let mut env = minijinja::Environment::new();
        // Install a filesystem loader rooted at ./templates so {% include %}
        // and {% extends %} can resolve sibling templates. Without this,
        // includes raise "template not found" — minijinja can't infer the
        // template root from a string-only render_string call. Using cwd
        // (matches how template.render loads templates: fs.read of
        // "templates/<name>.html").
        env.set_loader(minijinja::path_loader("templates"));
        let tmpl = env
            .template_from_str(&template_str)
            .map_err(|e| mlua::Error::runtime(format!("template.render_string: {e}")))?;
        tmpl.render(mini_vars)
            .map_err(|e| mlua::Error::runtime(format!("template.render_string: {e}")))
    })?;
    tmpl_table.set("render_string", render_string_fn)?;

    let render_fn = lua.create_function(|_, (file_path, vars): (String, Value)| {
        let content = std::fs::read_to_string(&file_path).map_err(|e| {
            mlua::Error::runtime(format!(
                "template.render: failed to read {file_path:?}: {e}"
            ))
        })?;
        let json_vars = match &vars {
            Value::Table(_) => lua_value_to_json(&vars)?,
            Value::Nil => serde_json::Value::Object(serde_json::Map::new()),
            _ => {
                return Err(mlua::Error::runtime(
                    "template.render: second argument must be a table or nil",
                ));
            }
        };
        let mini_vars = minijinja::value::Value::from_serialize(&json_vars);
        let env = minijinja::Environment::new();
        let tmpl = env
            .template_from_str(&content)
            .map_err(|e| mlua::Error::runtime(format!("template.render: {e}")))?;
        tmpl.render(mini_vars)
            .map_err(|e| mlua::Error::runtime(format!("template.render: {e}")))
    })?;
    tmpl_table.set("render", render_fn)?;

    let render_with_loader_fn = lua.create_function(
        |_, (template_dir, template_name, vars): (String, String, Value)| {
            let json_vars = match &vars {
                Value::Table(_) => lua_value_to_json(&vars)?,
                Value::Nil => serde_json::Value::Object(serde_json::Map::new()),
                _ => {
                    return Err(mlua::Error::runtime(
                        "template.render_with_loader: third argument must be a table or nil",
                    ));
                }
            };
            let mini_vars = minijinja::value::Value::from_serialize(&json_vars);
            let mut env = minijinja::Environment::new();
            env.set_loader(minijinja::path_loader(&template_dir));
            let tmpl = env
                .get_template(&template_name)
                .map_err(|e| mlua::Error::runtime(format!("template.render_with_loader: {e}")))?;
            tmpl.render(mini_vars)
                .map_err(|e| mlua::Error::runtime(format!("template.render_with_loader: {e}")))
        },
    )?;
    tmpl_table.set("render_with_loader", render_with_loader_fn)?;

    lua.globals().set("template", tmpl_table)?;
    Ok(())
}
