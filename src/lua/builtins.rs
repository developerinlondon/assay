use mlua::{Lua, Table, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

pub fn register_all(lua: &Lua, client: reqwest::Client) -> mlua::Result<()> {
    register_http(lua, client.clone())?;
    register_json(lua)?;
    register_assert(lua)?;
    register_log(lua)?;
    register_env(lua)?;
    register_sleep(lua)?;
    register_time(lua)?;
    register_prometheus(lua, client)?;
    Ok(())
}

fn register_http(lua: &Lua, client: reqwest::Client) -> mlua::Result<()> {
    let http_table = lua.create_table()?;

    let get_client = client.clone();
    let get_fn = lua.create_async_function(move |lua, args: mlua::MultiValue| {
        let client = get_client.clone();
        async move {
            let mut args_iter = args.into_iter();
            let url: String = match args_iter.next() {
                Some(Value::String(s)) => s.to_str()?.to_string(),
                _ => return Err(mlua::Error::runtime("http.get: first argument must be a URL string")),
            };

            let opts = match args_iter.next() {
                Some(Value::Table(t)) => Some(t),
                Some(Value::Nil) | None => None,
                _ => return Err(mlua::Error::runtime("http.get: second argument must be a table or nil")),
            };

            let mut req = client.get(&url);
            if let Some(ref opts_table) = opts
                && let Ok(headers_table) = opts_table.get::<Table>("headers")
            {
                for pair in headers_table.pairs::<String, String>() {
                    let (k, v) = pair?;
                    req = req.header(k, v);
                }
            }

            let resp = req.send().await.map_err(|e| mlua::Error::runtime(format!("http.get failed: {e}")))?;
            let status = resp.status().as_u16();
            let resp_headers = resp.headers().clone();
            let body = resp
                .text()
                .await
                .map_err(|e| mlua::Error::runtime(format!("http.get: reading body failed: {e}")))?;

            let result = lua.create_table()?;
            result.set("status", status)?;
            result.set("body", body)?;

            let headers_out = lua.create_table()?;
            for (name, value) in &resp_headers {
                if let Ok(v) = value.to_str() {
                    headers_out.set(name.as_str().to_string(), v.to_string())?;
                }
            }
            result.set("headers", headers_out)?;

            Ok(Value::Table(result))
        }
    })?;
    http_table.set("get", get_fn)?;

    let post_client = client;
    let post_fn = lua.create_async_function(move |lua, args: mlua::MultiValue| {
        let client = post_client.clone();
        async move {
            let mut args_iter = args.into_iter();
            let url: String = match args_iter.next() {
                Some(Value::String(s)) => s.to_str()?.to_string(),
                _ => return Err(mlua::Error::runtime("http.post: first argument must be a URL string")),
            };

            let (body_str, auto_json) = match args_iter.next() {
                Some(Value::String(s)) => (s.to_str()?.to_string(), false),
                Some(Value::Table(t)) => {
                    let json_val = lua_table_to_json(&t)?;
                    let serialized = serde_json::to_string(&json_val)
                        .map_err(|e| mlua::Error::runtime(format!("http.post: JSON encode failed: {e}")))?;
                    (serialized, true)
                }
                Some(Value::Nil) | None => (String::new(), false),
                _ => return Err(mlua::Error::runtime("http.post: second argument must be a string, table, or nil")),
            };

            let opts = match args_iter.next() {
                Some(Value::Table(t)) => Some(t),
                Some(Value::Nil) | None => None,
                _ => return Err(mlua::Error::runtime("http.post: third argument must be a table or nil")),
            };

            let mut req = client.post(&url).body(body_str);
            if auto_json {
                req = req.header("Content-Type", "application/json");
            }
            if let Some(ref opts_table) = opts
                && let Ok(headers_table) = opts_table.get::<Table>("headers")
            {
                for pair in headers_table.pairs::<String, String>() {
                    let (k, v) = pair?;
                    req = req.header(k, v);
                }
            }

            let resp = req.send().await.map_err(|e| mlua::Error::runtime(format!("http.post failed: {e}")))?;
            let status = resp.status().as_u16();
            let resp_headers = resp.headers().clone();
            let body = resp
                .text()
                .await
                .map_err(|e| mlua::Error::runtime(format!("http.post: reading body failed: {e}")))?;

            let result = lua.create_table()?;
            result.set("status", status)?;
            result.set("body", body)?;

            let headers_out = lua.create_table()?;
            for (name, value) in &resp_headers {
                if let Ok(v) = value.to_str() {
                    headers_out.set(name.as_str().to_string(), v.to_string())?;
                }
            }
            result.set("headers", headers_out)?;

            Ok(Value::Table(result))
        }
    })?;
    http_table.set("post", post_fn)?;

    lua.globals().set("http", http_table)?;
    Ok(())
}

fn register_json(lua: &Lua) -> mlua::Result<()> {
    let json_table = lua.create_table()?;

    let parse_fn = lua.create_function(|lua, s: String| {
        let value: serde_json::Value =
            serde_json::from_str(&s).map_err(|e| mlua::Error::runtime(format!("json.parse: {e}")))?;
        json_value_to_lua(lua, &value)
    })?;
    json_table.set("parse", parse_fn)?;

    let encode_fn = lua.create_function(|_, val: Value| {
        let json_val = lua_value_to_json(&val)?;
        serde_json::to_string(&json_val)
            .map_err(|e| mlua::Error::runtime(format!("json.encode: {e}")))
    })?;
    json_table.set("encode", encode_fn)?;

    lua.globals().set("json", json_table)?;
    Ok(())
}

fn lua_table_to_json(table: &mlua::Table) -> mlua::Result<serde_json::Value> {
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

    if is_array && max_index == count {
        let mut arr = Vec::with_capacity(max_index as usize);
        for i in 1..=max_index {
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

fn lua_value_to_json(val: &Value) -> mlua::Result<serde_json::Value> {
    match val {
        Value::Nil => Ok(serde_json::Value::Null),
        Value::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Integer(i) => Ok(serde_json::Value::Number(
            serde_json::Number::from(*i),
        )),
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

fn json_value_to_lua(lua: &Lua, val: &serde_json::Value) -> mlua::Result<Value> {
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

fn register_assert(lua: &Lua) -> mlua::Result<()> {
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
            _ => Err(mlua::Error::runtime("assert.gt: both arguments must be numbers")),
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
            _ => Err(mlua::Error::runtime("assert.lt: both arguments must be numbers")),
        }
    })?;
    assert_table.set("lt", lt_fn)?;

    let contains_fn = lua.create_function(|lua, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();
        let haystack: String = match args_iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => return Err(mlua::Error::runtime("assert.contains: first argument must be a string")),
        };
        let needle: String = match args_iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => return Err(mlua::Error::runtime("assert.contains: second argument must be a string")),
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
            _ => return Err(mlua::Error::runtime("assert.matches: first argument must be a string")),
        };
        let pattern: String = match args_iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => return Err(mlua::Error::runtime("assert.matches: second argument must be a pattern string")),
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

fn register_log(lua: &Lua) -> mlua::Result<()> {
    let log_table = lua.create_table()?;

    let info_fn = lua.create_function(|_, msg: String| {
        info!(target: "lua", "{}", msg);
        Ok(())
    })?;
    log_table.set("info", info_fn)?;

    let warn_fn = lua.create_function(|_, msg: String| {
        warn!(target: "lua", "{}", msg);
        Ok(())
    })?;
    log_table.set("warn", warn_fn)?;

    let error_fn = lua.create_function(|_, msg: String| {
        error!(target: "lua", "{}", msg);
        Ok(())
    })?;
    log_table.set("error", error_fn)?;

    lua.globals().set("log", log_table)?;
    Ok(())
}

fn register_env(lua: &Lua) -> mlua::Result<()> {
    let env_table = lua.create_table()?;

    let process_get_fn = lua.create_function(|_, name: String| match std::env::var(&name) {
        Ok(val) => Ok(Some(val)),
        Err(_) => Ok(None),
    })?;
    env_table.set("_process_get", process_get_fn)?;
    env_table.set("_check_env", lua.create_table()?)?;

    lua.globals().set("env", env_table)?;

    lua.load(
        r#"
        function env.get(name)
            local val = env._check_env[name]
            if val ~= nil then return val end
            return env._process_get(name)
        end
        "#,
    )
    .exec()?;

    Ok(())
}

fn register_sleep(lua: &Lua) -> mlua::Result<()> {
    let sleep_fn = lua.create_async_function(|_, seconds: f64| async move {
        let duration = std::time::Duration::from_secs_f64(seconds);
        tokio::time::sleep(duration).await;
        Ok(())
    })?;
    lua.globals().set("sleep", sleep_fn)?;
    Ok(())
}

fn register_time(lua: &Lua) -> mlua::Result<()> {
    let time_fn = lua.create_function(|_, ()| {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| mlua::Error::runtime(format!("time(): {e}")))?
            .as_secs_f64();
        Ok(secs)
    })?;
    lua.globals().set("time", time_fn)?;
    Ok(())
}

fn register_prometheus(lua: &Lua, client: reqwest::Client) -> mlua::Result<()> {
    let prom_table = lua.create_table()?;

    let query_fn = lua.create_async_function(move |lua, (url, promql): (String, String)| {
        let client = client.clone();
        async move {
            let query_url = format!("{}/api/v1/query", url.trim_end_matches('/'));
            let resp = client
                .get(&query_url)
                .query(&[("query", &promql)])
                .send()
                .await
                .map_err(|e| mlua::Error::runtime(format!("prometheus.query: request failed: {e}")))?;

            let body = resp
                .text()
                .await
                .map_err(|e| mlua::Error::runtime(format!("prometheus.query: reading body failed: {e}")))?;

            let parsed: serde_json::Value = serde_json::from_str(&body)
                .map_err(|e| mlua::Error::runtime(format!("prometheus.query: invalid JSON: {e}")))?;

            let result_array = parsed
                .get("data")
                .and_then(|d| d.get("result"))
                .and_then(|r| r.as_array());

            match result_array {
                Some(results) if results.len() == 1 => {
                    let val_str = results[0]
                        .get("value")
                        .and_then(|v| v.as_array())
                        .and_then(|arr| arr.get(1))
                        .and_then(|v| v.as_str())
                        .unwrap_or("0");

                    match val_str.parse::<f64>() {
                        Ok(num) => Ok(Value::Number(num)),
                        Err(_) => Ok(Value::String(lua.create_string(val_str)?)),
                    }
                }
                Some(results) => {
                    let table = lua.create_table()?;
                    for (i, result) in results.iter().enumerate() {
                        let row = lua.create_table()?;

                        if let Some(metric) = result.get("metric") {
                            let metric_table = json_value_to_lua(&lua, metric)?;
                            row.set("metric", metric_table)?;
                        }

                        let val_str = result
                            .get("value")
                            .and_then(|v| v.as_array())
                            .and_then(|arr| arr.get(1))
                            .and_then(|v| v.as_str())
                            .unwrap_or("0");
                        match val_str.parse::<f64>() {
                            Ok(num) => row.set("value", num)?,
                            Err(_) => row.set("value", val_str)?,
                        }

                        table.set(i + 1, row)?;
                    }
                    Ok(Value::Table(table))
                }
                None => Err(mlua::Error::runtime(format!(
                    "prometheus.query: unexpected response format: {body}"
                ))),
            }
        }
    })?;
    prom_table.set("query", query_fn)?;

    lua.globals().set("prometheus", prom_table)?;
    Ok(())
}

fn extract_string_arg(lua: &Lua, val: Option<Value>) -> Option<String> {
    match val {
        Some(Value::String(s)) => s.to_str().ok().map(|s| s.to_string()),
        Some(other) => {
            let result: Option<String> = lua
                .load("return tostring(...)")
                .call(other)
                .ok();
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
