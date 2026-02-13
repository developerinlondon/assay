use mlua::{Lua, Value};

pub fn register_assert(lua: &Lua) -> mlua::Result<()> {
    let assert_table = lua.create_table()?;

    let eq_fn = lua.create_function(|lua, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();
        let a = args_iter.next().unwrap_or(Value::Nil);
        let b = args_iter.next().unwrap_or(Value::Nil);
        let msg = extract_string_arg(lua, args_iter.next());

        if !lua_values_equal(&a, &b) {
            let detail = format!(
                "assert.eq failed: {:?} != {:?}{}",
                format_lua_value(&a),
                format_lua_value(&b),
                msg.map(|m| format!(" - {m}")).unwrap_or_default()
            );
            return Err(mlua::Error::runtime(detail));
        }
        Ok(())
    })?;
    assert_table.set("eq", eq_fn)?;

    let gt_fn = lua.create_function(|lua, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();
        let a = lua_value_to_f64(args_iter.next().unwrap_or(Value::Nil));
        let b = lua_value_to_f64(args_iter.next().unwrap_or(Value::Nil));
        let msg = extract_string_arg(lua, args_iter.next());

        match (a, b) {
            (Some(va), Some(vb)) if va > vb => Ok(()),
            (Some(va), Some(vb)) => Err(mlua::Error::runtime(format!(
                "assert.gt failed: {va} is not > {vb}{}",
                msg.map(|m| format!(" - {m}")).unwrap_or_default()
            ))),
            _ => Err(mlua::Error::runtime(
                "assert.gt: both arguments must be numbers",
            )),
        }
    })?;
    assert_table.set("gt", gt_fn)?;

    let lt_fn = lua.create_function(|lua, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();
        let a = lua_value_to_f64(args_iter.next().unwrap_or(Value::Nil));
        let b = lua_value_to_f64(args_iter.next().unwrap_or(Value::Nil));
        let msg = extract_string_arg(lua, args_iter.next());

        match (a, b) {
            (Some(va), Some(vb)) if va < vb => Ok(()),
            (Some(va), Some(vb)) => Err(mlua::Error::runtime(format!(
                "assert.lt failed: {va} is not < {vb}{}",
                msg.map(|m| format!(" - {m}")).unwrap_or_default()
            ))),
            _ => Err(mlua::Error::runtime(
                "assert.lt: both arguments must be numbers",
            )),
        }
    })?;
    assert_table.set("lt", lt_fn)?;

    let contains_fn = lua.create_function(|lua, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();
        let haystack: String = match args_iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => {
                return Err(mlua::Error::runtime(
                    "assert.contains: first argument must be a string",
                ));
            }
        };
        let needle: String = match args_iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => {
                return Err(mlua::Error::runtime(
                    "assert.contains: second argument must be a string",
                ));
            }
        };
        let msg = extract_string_arg(lua, args_iter.next());

        if !haystack.contains(&needle) {
            return Err(mlua::Error::runtime(format!(
                "assert.contains failed: {haystack:?} does not contain {needle:?}{}",
                msg.map(|m| format!(" - {m}")).unwrap_or_default()
            )));
        }
        Ok(())
    })?;
    assert_table.set("contains", contains_fn)?;

    let not_nil_fn = lua.create_function(|lua, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();
        let val = args_iter.next().unwrap_or(Value::Nil);
        let msg = extract_string_arg(lua, args_iter.next());

        if val == Value::Nil {
            return Err(mlua::Error::runtime(format!(
                "assert.not_nil failed: value is nil{}",
                msg.map(|m| format!(" - {m}")).unwrap_or_default()
            )));
        }
        Ok(())
    })?;
    assert_table.set("not_nil", not_nil_fn)?;

    let matches_fn = lua.create_function(|lua, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();
        let text: String = match args_iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => {
                return Err(mlua::Error::runtime(
                    "assert.matches: first argument must be a string",
                ));
            }
        };
        let pattern: String = match args_iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => {
                return Err(mlua::Error::runtime(
                    "assert.matches: second argument must be a pattern string",
                ));
            }
        };
        let msg = extract_string_arg(lua, args_iter.next());

        let found: bool = lua
            .load(format!(
                "return string.find({}, {}) ~= nil",
                lua_string_literal(&text),
                lua_string_literal(&pattern)
            ))
            .eval()
            .map_err(|e| mlua::Error::runtime(format!("assert.matches: pattern error: {e}")))?;

        if !found {
            return Err(mlua::Error::runtime(format!(
                "assert.matches failed: {text:?} does not match pattern {pattern:?}{}",
                msg.map(|m| format!(" - {m}")).unwrap_or_default()
            )));
        }
        Ok(())
    })?;
    assert_table.set("matches", matches_fn)?;

    lua.globals().set("assert", assert_table)?;
    Ok(())
}

fn extract_string_arg(lua: &Lua, val: Option<Value>) -> Option<String> {
    match val {
        Some(Value::String(s)) => s.to_str().ok().map(|s| s.to_string()),
        Some(other) => {
            let result: Option<String> = lua.load("return tostring(...)").call(other).ok();
            result
        }
        None => None,
    }
}

fn lua_values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Nil, Value::Nil) => true,
        (Value::Boolean(a), Value::Boolean(b)) => a == b,
        (Value::Integer(a), Value::Integer(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => (a - b).abs() < f64::EPSILON,
        (Value::Integer(a), Value::Number(b)) | (Value::Number(b), Value::Integer(a)) => {
            (*a as f64 - b).abs() < f64::EPSILON
        }
        (Value::String(a), Value::String(b)) => a.as_bytes() == b.as_bytes(),
        _ => false,
    }
}

fn lua_value_to_f64(val: Value) -> Option<f64> {
    match val {
        Value::Integer(i) => Some(i as f64),
        Value::Number(f) => Some(f),
        _ => None,
    }
}

fn format_lua_value(val: &Value) -> String {
    match val {
        Value::Nil => "nil".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(f) => f.to_string(),
        Value::String(s) => match s.to_str() {
            Ok(v) => v.to_string(),
            Err(_) => "<invalid utf-8>".to_string(),
        },
        _ => format!("<{}>", val.type_name()),
    }
}

fn lua_string_literal(s: &str) -> String {
    let escaped = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\0', "\\0");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lua_values_equal_nil() {
        assert!(lua_values_equal(&Value::Nil, &Value::Nil));
    }

    #[test]
    fn test_lua_values_equal_int_float() {
        assert!(lua_values_equal(&Value::Integer(42), &Value::Number(42.0)));
    }

    #[test]
    fn test_lua_values_not_equal() {
        assert!(!lua_values_equal(&Value::Integer(1), &Value::Integer(2)));
    }

    #[test]
    fn test_format_lua_value() {
        assert_eq!(format_lua_value(&Value::Nil), "nil");
        assert_eq!(format_lua_value(&Value::Boolean(true)), "true");
        assert_eq!(format_lua_value(&Value::Integer(42)), "42");
    }

    #[test]
    fn test_lua_string_literal_escaping() {
        assert_eq!(lua_string_literal("hello"), "\"hello\"");
        assert_eq!(lua_string_literal("line\nnew"), "\"line\\nnew\"");
        assert_eq!(lua_string_literal("quote\"here"), "\"quote\\\"here\"");
    }
}
