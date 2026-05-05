use mlua::{Lua, Value};

pub fn register_json(lua: &Lua) -> mlua::Result<()> {
    let json_table = lua.create_table()?;

    let parse_fn = lua.create_function(|lua, s: String| {
        let value: serde_json::Value = serde_json::from_str(&s)
            .map_err(|e| mlua::Error::runtime(format!("json.parse: {e}")))?;
        json_value_to_lua(lua, &value)
    })?;
    json_table.set("parse", parse_fn)?;

    let encode_fn = lua.create_function(|_, val: Value| {
        let json_val = lua_value_to_json(&val)?;
        serde_json::to_string(&json_val)
            .map_err(|e| mlua::Error::runtime(format!("json.encode: {e}")))
    })?;
    json_table.set("encode", encode_fn)?;

    // json.array(t) / json.object(t) — escape hatches that pin the shape
    // a table will encode to, regardless of its contents. They attach a
    // `__jsontype` metatable marker that `lua_table_to_json` honours.
    //
    // Needed because Lua has one composite type (table) covering both
    // arrays and objects, so the encoder has to guess intent. The empty
    // case is genuinely ambiguous and the default (empty → object) is
    // wrong for the rare caller that wants `[]`. Use `json.array({})` in
    // that case. Symmetrically, `json.object` lets you force object
    // shape on a sequential 1..N table — useful when an API expects a
    // JSON object whose keys happen to be string-encoded integers.
    let array_fn = lua.create_function(|lua, t: Option<mlua::Table>| {
        let table = t.unwrap_or(lua.create_table()?);
        attach_jsontype(lua, &table, "array")?;
        Ok(table)
    })?;
    json_table.set("array", array_fn)?;

    let object_fn = lua.create_function(|lua, t: Option<mlua::Table>| {
        let table = t.unwrap_or(lua.create_table()?);
        attach_jsontype(lua, &table, "object")?;
        Ok(table)
    })?;
    json_table.set("object", object_fn)?;

    lua.globals().set("json", json_table)?;
    Ok(())
}

// Set or update the `__jsontype` field on a table's metatable. Preserves
// any existing metatable so a caller that already has __index / __tostring
// etc. doesn't lose them.
fn attach_jsontype(lua: &Lua, table: &mlua::Table, kind: &str) -> mlua::Result<()> {
    let mt = match table.metatable() {
        Some(existing) => existing,
        None => {
            let new = lua.create_table()?;
            table.set_metatable(Some(new.clone()))?;
            new
        }
    };
    mt.set("__jsontype", kind)?;
    Ok(())
}

// Read the `__jsontype` marker from a table's metatable. Returns
// "array" / "object" if explicitly set, None otherwise.
fn jsontype_marker(table: &mlua::Table) -> Option<String> {
    let mt = table.metatable()?;
    match mt.get::<Value>("__jsontype").ok()? {
        Value::String(s) => s.to_str().ok().map(|s| s.to_string()),
        _ => None,
    }
}

pub fn lua_table_to_json(table: &mlua::Table) -> mlua::Result<serde_json::Value> {
    // Honour an explicit __jsontype marker before doing any heuristic.
    // Lets callers force "object" on a 1..N table or "array" on an empty
    // / sparse one, which the heuristic alone can't express.
    let forced_kind = jsontype_marker(table);

    let mut is_array = true;
    let mut max_index: i64 = 0;
    let mut count: i64 = 0;

    for pair in table.clone().pairs::<Value, Value>() {
        let (key, _) = pair?;
        count += 1;
        match key {
            Value::Integer(i) if i >= 1 => {
                if i > max_index {
                    max_index = i;
                }
            }
            _ => {
                is_array = false;
                break;
            }
        }
    }

    // Decide the final shape. Order:
    //   1. Explicit __jsontype marker wins.
    //   2. Non-empty 1..N integer keys → array (existing behaviour).
    //   3. Anything else → object. This includes the empty table, where
    //      historically we emitted `[]` — which broke every assay caller
    //      passing `{}` as a JSON request body, since "object" is what
    //      they all wanted. See issue #129.
    let encode_as_array = match forced_kind.as_deref() {
        Some("array") => true,
        Some("object") => false,
        _ => is_array && count > 0 && max_index == count,
    };

    if encode_as_array {
        let len = if is_array && count > 0 && max_index == count {
            max_index
        } else {
            count
        };
        let mut arr = Vec::with_capacity(len.max(0) as usize);
        for i in 1..=len {
            let val: Value = table.get(i)?;
            arr.push(lua_value_to_json(&val)?);
        }
        Ok(serde_json::Value::Array(arr))
    } else {
        let mut map = serde_json::Map::new();
        for pair in table.clone().pairs::<Value, Value>() {
            let (key, val) = pair?;
            let key_str = match key {
                Value::String(s) => s.to_str()?.to_string(),
                Value::Integer(i) => i.to_string(),
                Value::Number(f) => f.to_string(),
                _ => {
                    return Err(mlua::Error::runtime(format!(
                        "unsupported table key type: {}",
                        key.type_name()
                    )));
                }
            };
            map.insert(key_str, lua_value_to_json(&val)?);
        }
        Ok(serde_json::Value::Object(map))
    }
}

pub fn lua_value_to_json(val: &Value) -> mlua::Result<serde_json::Value> {
    match val {
        Value::Nil => Ok(serde_json::Value::Null),
        Value::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Integer(i) => Ok(serde_json::Value::Number(serde_json::Number::from(*i))),
        Value::Number(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .ok_or_else(|| mlua::Error::runtime(format!("cannot encode {f} as JSON number"))),
        Value::String(s) => Ok(serde_json::Value::String(s.to_str()?.to_string())),
        Value::Table(t) => lua_table_to_json(t),
        _ => Err(mlua::Error::runtime(format!(
            "unsupported Lua type for JSON: {}",
            val.type_name()
        ))),
    }
}

pub fn json_value_to_lua(lua: &Lua, val: &serde_json::Value) -> mlua::Result<Value> {
    match val {
        serde_json::Value::Null => Ok(Value::Nil),
        serde_json::Value::Bool(b) => Ok(Value::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Number(f))
            } else {
                Ok(Value::Nil)
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(lua.create_string(s)?)),
        serde_json::Value::Array(arr) => {
            let table = lua.create_table()?;
            for (i, item) in arr.iter().enumerate() {
                table.set(i + 1, json_value_to_lua(lua, item)?)?;
            }
            Ok(Value::Table(table))
        }
        serde_json::Value::Object(map) => {
            let table = lua.create_table()?;
            for (k, v) in map {
                table.set(k.as_str(), json_value_to_lua(lua, v)?)?;
            }
            Ok(Value::Table(table))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lua_value_to_json_nil() {
        let result = lua_value_to_json(&Value::Nil).unwrap();
        assert_eq!(result, serde_json::Value::Null);
    }

    #[test]
    fn test_lua_value_to_json_bool() {
        assert_eq!(
            lua_value_to_json(&Value::Boolean(true)).unwrap(),
            serde_json::Value::Bool(true)
        );
    }

    #[test]
    fn test_lua_value_to_json_integer() {
        assert_eq!(
            lua_value_to_json(&Value::Integer(42)).unwrap(),
            serde_json::json!(42)
        );
    }

    #[test]
    fn test_lua_value_to_json_number() {
        assert_eq!(
            lua_value_to_json(&Value::Number(1.5)).unwrap(),
            serde_json::json!(1.5)
        );
    }
}
