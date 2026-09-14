use super::json::{json_value_to_lua, lua_value_to_json};
use mlua::{Lua, Table, Value};
use serde::Deserialize;
use serde::de::{self, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor};
use serde_yml::value::{Tag, TaggedValue};
use std::fmt;

/// What a YAML tag becomes on the Lua side.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TagMode {
    /// A one-key table keyed by the tag text: `{ ["!reference"] = payload }`.
    Wrap,
    /// No tag semantics: on parse the payload alone, on encode a `!` key is an
    /// ordinary mapping key.
    Strip,
}

/// Read `{ tags = "wrap" | "strip" }` off an optional options table. The two
/// directions default differently: parsing wraps, because a refused document is
/// the bug being fixed, while encoding strips, because reading a `!` key as a
/// tag would change what an existing caller's table encodes to.
fn tag_mode(fname: &str, options: Option<&Table>, default: TagMode) -> mlua::Result<TagMode> {
    let Some(options) = options else {
        return Ok(default);
    };
    match options.get::<Value>("tags")? {
        Value::Nil => Ok(default),
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

/// A parsed YAML document: serde_json's scalar model, so number widening and
/// non-finite floats stay what they were when this deserialized into
/// `serde_json::Value`, plus the tagged node JSON has no room for. A mapping is
/// a list because its keys are read as strings, so a scalar key arrives as the
/// text the document wrote (`~` stays `~`, `1.50` stays `1.50`), and duplicates
/// stay in order so the later one wins once the Lua table is built.
enum YamlNode {
    Scalar(serde_json::Value),
    Sequence(Vec<YamlNode>),
    Mapping(Vec<(String, YamlNode)>),
    Tagged(String, Box<YamlNode>),
}

impl<'de> Deserialize<'de> for YamlNode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct NodeVisitor;

        impl<'de> Visitor<'de> for NodeVisitor {
            type Value = YamlNode;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("any YAML value")
            }

            fn visit_unit<E: de::Error>(self) -> Result<YamlNode, E> {
                Ok(YamlNode::Scalar(serde_json::Value::Null))
            }

            fn visit_none<E: de::Error>(self) -> Result<YamlNode, E> {
                Ok(YamlNode::Scalar(serde_json::Value::Null))
            }

            fn visit_some<D>(self, deserializer: D) -> Result<YamlNode, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                Deserialize::deserialize(deserializer)
            }

            fn visit_bool<E: de::Error>(self, v: bool) -> Result<YamlNode, E> {
                Ok(YamlNode::Scalar(v.into()))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<YamlNode, E> {
                Ok(YamlNode::Scalar(v.into()))
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<YamlNode, E> {
                Ok(YamlNode::Scalar(v.into()))
            }

            fn visit_f64<E: de::Error>(self, v: f64) -> Result<YamlNode, E> {
                Ok(YamlNode::Scalar(v.into()))
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<YamlNode, E> {
                Ok(YamlNode::Scalar(v.into()))
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<YamlNode, E> {
                Ok(YamlNode::Scalar(v.into()))
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<YamlNode, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(YamlNode::Sequence(items))
            }

            fn visit_map<A>(self, mut map: A) -> Result<YamlNode, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry::<String, YamlNode>()? {
                    entries.push(entry);
                }
                Ok(YamlNode::Mapping(entries))
            }

            fn visit_enum<A>(self, data: A) -> Result<YamlNode, A::Error>
            where
                A: EnumAccess<'de>,
            {
                let (tag, contents) = data.variant::<String>()?;
                if tag.is_empty() {
                    return Err(de::Error::custom("empty YAML tag is not allowed"));
                }
                let payload = contents.newtype_variant::<YamlNode>()?;
                Ok(YamlNode::Tagged(
                    Tag::new(tag).to_string(),
                    Box::new(payload),
                ))
            }
        }

        deserializer.deserialize_any(NodeVisitor)
    }
}

fn yaml_node_to_lua(lua: &Lua, node: &YamlNode, mode: TagMode) -> mlua::Result<Value> {
    match node {
        YamlNode::Scalar(val) => json_value_to_lua(lua, val),
        YamlNode::Sequence(items) => {
            let table = lua.create_table()?;
            for (i, item) in items.iter().enumerate() {
                table.set(i + 1, yaml_node_to_lua(lua, item, mode)?)?;
            }
            Ok(Value::Table(table))
        }
        YamlNode::Mapping(entries) => {
            let table = lua.create_table()?;
            for (key, val) in entries {
                table.set(key.as_str(), yaml_node_to_lua(lua, val, mode)?)?;
            }
            Ok(Value::Table(table))
        }
        YamlNode::Tagged(tag, payload) => match mode {
            TagMode::Strip => yaml_node_to_lua(lua, payload, mode),
            TagMode::Wrap => {
                let table = lua.create_table()?;
                // A Lua table cannot hold a nil value, so a tag over an empty
                // payload would wrap to an empty table and lose the tag. The
                // empty string stands in for it and round-trips.
                let value = match yaml_node_to_lua(lua, payload, mode)? {
                    Value::Nil => Value::String(lua.create_string("")?),
                    value => value,
                };
                table.set(tag.as_str(), value)?;
                Ok(Value::Table(table))
            }
        },
    }
}

/// Entries are inserted in `serde_json::Map` order, which is sorted, and
/// `serde_yml::Mapping` preserves insertion order, so key ordering survives.
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
        let mode = tag_mode("yaml.parse", options.as_ref(), TagMode::Wrap)?;
        let node: YamlNode = serde_yml::from_str(&s)
            .map_err(|e| mlua::Error::runtime(format!("yaml.parse: {e}")))?;
        yaml_node_to_lua(lua, &node, mode)
    })?;
    yaml_table.set("parse", parse_fn)?;

    let parse_all_fn = lua.create_function(|lua, (s, options): (String, Option<Table>)| {
        let mode = tag_mode("yaml.parse_all", options.as_ref(), TagMode::Wrap)?;
        let docs = lua.create_table()?;
        let mut index = 1;
        for doc in serde_yml::Deserializer::from_str(&s) {
            let node = YamlNode::deserialize(doc)
                .map_err(|e| mlua::Error::runtime(format!("yaml.parse_all: {e}")))?;
            if !matches!(node, YamlNode::Scalar(serde_json::Value::Null)) {
                docs.set(index, yaml_node_to_lua(lua, &node, mode)?)?;
                index += 1;
            }
        }
        Ok(Value::Table(docs))
    })?;
    yaml_table.set("parse_all", parse_all_fn)?;

    let encode_fn = lua.create_function(|_, (val, options): (Value, Option<Table>)| {
        let mode = tag_mode("yaml.encode", options.as_ref(), TagMode::Strip)?;
        let json_val = lua_value_to_json(&val)?;
        let encoded = match mode {
            TagMode::Strip => serde_yml::to_string(&json_val),
            TagMode::Wrap => serde_yml::to_string(&json_value_to_yaml(json_val)),
        };
        encoded.map_err(|e| mlua::Error::runtime(format!("yaml.encode: {e}")))
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
