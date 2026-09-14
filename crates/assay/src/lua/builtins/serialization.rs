use super::json::{json_value_to_lua, lua_value_to_json};
use mlua::{Lua, Table, Value};
use serde::Deserialize;
use serde_yml::value::{Tag, TaggedValue};

/// What a YAML tag becomes on the Lua side.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TagMode {
    /// A one-key table keyed by the tag text: `{ ["!reference"] = payload }`.
    Wrap,
    /// The payload alone, as if the node carried no tag.
    Strip,
}

/// Read `{ tags = "wrap" | "strip" }` off an optional options table.
fn tag_mode(fname: &str, options: Option<&Table>) -> mlua::Result<TagMode> {
    let Some(options) = options else {
        return Ok(TagMode::Wrap);
    };
    match options.get::<Value>("tags")? {
        Value::Nil => Ok(TagMode::Wrap),
        Value::String(s) => match s.to_str()?.to_string().as_str() {
            "wrap" => Ok(TagMode::Wrap),
            "strip" => Ok(TagMode::Strip),
            other => Err(mlua::Error::runtime(format!(
                "{fname}: tags must be \"wrap\" or \"strip\", got \"{other}\""
            ))),
        },
        other => Err(mlua::Error::runtime(format!(
            "{fname}: tags must be \"wrap\" or \"strip\", got {}",
            other.type_name()
        ))),
    }
}

fn yaml_value_to_lua(
    lua: &Lua,
    val: &serde_yml::Value,
    mode: TagMode,
    fname: &str,
) -> mlua::Result<Value> {
    match val {
        serde_yml::Value::Null => Ok(Value::Nil),
        serde_yml::Value::Bool(b) => Ok(Value::Boolean(*b)),
        serde_yml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Number(f))
            } else {
                Ok(Value::Nil)
            }
        }
        serde_yml::Value::String(s) => Ok(Value::String(lua.create_string(s)?)),
        serde_yml::Value::Sequence(items) => {
            let table = lua.create_table()?;
            for (i, item) in items.iter().enumerate() {
                table.set(i + 1, yaml_value_to_lua(lua, item, mode, fname)?)?;
            }
            Ok(Value::Table(table))
        }
        serde_yml::Value::Mapping(map) => {
            let table = lua.create_table()?;
            for (key, val) in map {
                let serde_yml::Value::String(key) = key else {
                    return Err(mlua::Error::runtime(format!(
                        "{fname}: unsupported YAML mapping key: expected a string"
                    )));
                };
                table.set(key.as_str(), yaml_value_to_lua(lua, val, mode, fname)?)?;
            }
            Ok(Value::Table(table))
        }
        serde_yml::Value::Tagged(tagged) => match mode {
            TagMode::Strip => yaml_value_to_lua(lua, &tagged.value, mode, fname),
            TagMode::Wrap => {
                let table = lua.create_table()?;
                table.set(
                    tagged.tag.to_string(),
                    yaml_value_to_lua(lua, &tagged.value, mode, fname)?,
                )?;
                Ok(Value::Table(table))
            }
        },
    }
}

/// Encoding goes Lua → `serde_json::Value` → here, rather than straight to a
/// YAML value, so that every untagged document emits exactly the bytes it
/// emitted before tags were understood: `serde_json::Map` iterates its keys
/// sorted, `serde_yml::Mapping` preserves insertion order, and feeding the
/// former into the latter carries that ordering across unchanged.
fn json_value_to_yaml(val: serde_json::Value) -> serde_yml::Value {
    match val {
        serde_json::Value::Null => serde_yml::Value::Null,
        serde_json::Value::Bool(b) => serde_yml::Value::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_yml::Value::Number(i.into())
            } else if let Some(f) = n.as_f64() {
                serde_yml::Value::Number(f.into())
            } else {
                serde_yml::Value::Null
            }
        }
        serde_json::Value::String(s) => serde_yml::Value::String(s),
        serde_json::Value::Array(items) => {
            serde_yml::Value::Sequence(items.into_iter().map(json_value_to_yaml).collect())
        }
        serde_json::Value::Object(map) => {
            let mut entries = map.into_iter();
            match (entries.next(), entries.next()) {
                (Some((key, val)), None) if key.starts_with('!') => {
                    serde_yml::Value::Tagged(Box::new(TaggedValue {
                        tag: Tag::new(key),
                        value: json_value_to_yaml(val),
                    }))
                }
                (first, second) => {
                    let mut mapping = serde_yml::Mapping::new();
                    for (key, val) in first.into_iter().chain(second).chain(entries) {
                        mapping.insert(serde_yml::Value::String(key), json_value_to_yaml(val));
                    }
                    serde_yml::Value::Mapping(mapping)
                }
            }
        }
    }
}

pub fn register_yaml(lua: &Lua) -> mlua::Result<()> {
    let yaml_table = lua.create_table()?;

    let parse_fn = lua.create_function(|lua, (s, options): (String, Option<Table>)| {
        let mode = tag_mode("yaml.parse", options.as_ref())?;
        let yaml_val: serde_yml::Value = serde_yml::from_str(&s)
            .map_err(|e| mlua::Error::runtime(format!("yaml.parse: {e}")))?;
        yaml_value_to_lua(lua, &yaml_val, mode, "yaml.parse")
    })?;
    yaml_table.set("parse", parse_fn)?;

    let parse_all_fn = lua.create_function(|lua, (s, options): (String, Option<Table>)| {
        let mode = tag_mode("yaml.parse_all", options.as_ref())?;
        let docs = lua.create_table()?;
        let mut index = 1;
        for doc in serde_yml::Deserializer::from_str(&s) {
            let yaml_val = serde_yml::Value::deserialize(doc)
                .map_err(|e| mlua::Error::runtime(format!("yaml.parse_all: {e}")))?;
            if !matches!(yaml_val, serde_yml::Value::Null) {
                docs.set(
                    index,
                    yaml_value_to_lua(lua, &yaml_val, mode, "yaml.parse_all")?,
                )?;
                index += 1;
            }
        }
        Ok(Value::Table(docs))
    })?;
    yaml_table.set("parse_all", parse_all_fn)?;

    let encode_fn = lua.create_function(|_, val: Value| {
        let yaml_val = json_value_to_yaml(lua_value_to_json(&val)?);
        serde_yml::to_string(&yaml_val)
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
