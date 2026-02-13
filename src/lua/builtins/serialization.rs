use super::json::{json_value_to_lua, lua_value_to_json};
use mlua::{Lua, Value};

pub fn register_yaml(lua: &Lua) -> mlua::Result<()> {
    let yaml_table = lua.create_table()?;

    let parse_fn = lua.create_function(|lua, s: String| {
        let json_val: serde_json::Value = serde_yml::from_str(&s)
            .map_err(|e| mlua::Error::runtime(format!("yaml.parse: {e}")))?;
        json_value_to_lua(lua, &json_val)
    })?;
    yaml_table.set("parse", parse_fn)?;

    let encode_fn = lua.create_function(|_, val: Value| {
        let json_val = lua_value_to_json(&val)?;
        serde_yml::to_string(&json_val)
            .map_err(|e| mlua::Error::runtime(format!("yaml.encode: {e}")))
    })?;
    yaml_table.set("encode", encode_fn)?;

    lua.globals().set("yaml", yaml_table)?;
    Ok(())
}

pub fn register_toml(lua: &Lua) -> mlua::Result<()> {
    let toml_table = lua.create_table()?;

    let parse_fn = lua.create_function(|lua, s: String| {
        let toml_val: toml::Value =
            toml::from_str(&s).map_err(|e| mlua::Error::runtime(format!("toml.parse: {e}")))?;
        let json_val = serde_json::to_value(&toml_val)
            .map_err(|e| mlua::Error::runtime(format!("toml.parse: conversion failed: {e}")))?;
        json_value_to_lua(lua, &json_val)
    })?;
    toml_table.set("parse", parse_fn)?;

    let encode_fn = lua.create_function(|_, val: Value| {
        let json_val = lua_value_to_json(&val)?;
        let toml_val: toml::Value = serde_json::from_value(json_val)
            .map_err(|e| mlua::Error::runtime(format!("toml.encode: {e}")))?;
        toml::to_string_pretty(&toml_val)
            .map_err(|e| mlua::Error::runtime(format!("toml.encode: {e}")))
    })?;
    toml_table.set("encode", encode_fn)?;

    lua.globals().set("toml", toml_table)?;
    Ok(())
}
