use data_encoding::BASE64;
use digest::Digest;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use mlua::{Lua, Table, Value};
use rand::RngExt;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};
use zeroize::Zeroizing;

pub fn register_all(lua: &Lua, client: reqwest::Client) -> mlua::Result<()> {
    register_http(lua, client)?;
    register_json(lua)?;
    register_assert(lua)?;
    register_log(lua)?;
    register_env(lua)?;
    register_sleep(lua)?;
    register_time(lua)?;
    register_fs(lua)?;
    register_base64(lua)?;
    register_crypto(lua)?;
    register_regex(lua)?;
    Ok(())
}

fn register_http(lua: &Lua, client: reqwest::Client) -> mlua::Result<()> {
    let http_table = lua.create_table()?;

    for method in ["get", "post", "put", "patch", "delete"] {
        let method_client = client.clone();
        let method_name = method.to_string();
        let has_body = method != "get" && method != "delete";

        let func = lua.create_async_function(move |lua, args: mlua::MultiValue| {
            let client = method_client.clone();
            let method_name = method_name.clone();
            async move {
                let mut args_iter = args.into_iter();
                let url: String = match args_iter.next() {
                    Some(Value::String(s)) => s.to_str()?.to_string(),
                    _ => {
                        return Err(mlua::Error::runtime(format!(
                            "http.{method_name}: first argument must be a URL string"
                        )));
                    }
                };

                let (body_str, auto_json, opts) = if has_body {
                    let (body, is_json) = match args_iter.next() {
                        Some(Value::String(s)) => (s.to_str()?.to_string(), false),
                        Some(Value::Table(t)) => {
                            let json_val = lua_table_to_json(&t)?;
                            let serialized = serde_json::to_string(&json_val).map_err(|e| {
                                mlua::Error::runtime(format!(
                                    "http.{method_name}: JSON encode failed: {e}"
                                ))
                            })?;
                            (serialized, true)
                        }
                        Some(Value::Nil) | None => (String::new(), false),
                        _ => {
                            return Err(mlua::Error::runtime(format!(
                                "http.{method_name}: second argument must be a string, table, or nil"
                            )));
                        }
                    };
                    let opts = match args_iter.next() {
                        Some(Value::Table(t)) => Some(t),
                        Some(Value::Nil) | None => None,
                        _ => {
                            return Err(mlua::Error::runtime(format!(
                                "http.{method_name}: third argument must be a table or nil"
                            )));
                        }
                    };
                    (body, is_json, opts)
                } else {
                    let opts = match args_iter.next() {
                        Some(Value::Table(t)) => Some(t),
                        Some(Value::Nil) | None => None,
                        _ => {
                            return Err(mlua::Error::runtime(format!(
                                "http.{method_name}: second argument must be a table or nil"
                            )));
                        }
                    };
                    (String::new(), false, opts)
                };

                let mut req = match method_name.as_str() {
                    "get" => client.get(&url),
                    "post" => client.post(&url),
                    "put" => client.put(&url),
                    "patch" => client.patch(&url),
                    "delete" => client.delete(&url),
                    _ => unreachable!(),
                };

                if has_body && !body_str.is_empty() {
                    req = req.body(body_str);
                }
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

                let resp = req.send().await.map_err(|e| {
                    mlua::Error::runtime(format!("http.{method_name} failed: {e}"))
                })?;
                let status = resp.status().as_u16();
                let resp_headers = resp.headers().clone();
                let body = resp.text().await.map_err(|e| {
                    mlua::Error::runtime(format!(
                        "http.{method_name}: reading body failed: {e}"
                    ))
                })?;

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
        http_table.set(method, func)?;
    }

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

fn register_fs(lua: &Lua) -> mlua::Result<()> {
    let fs_table = lua.create_table()?;

    let read_fn = lua.create_function(|_, path: String| {
        std::fs::read_to_string(&path)
            .map_err(|e| mlua::Error::runtime(format!("fs.read: failed to read {path:?}: {e}")))
    })?;
    fs_table.set("read", read_fn)?;

    lua.globals().set("fs", fs_table)?;
    Ok(())
}

fn register_base64(lua: &Lua) -> mlua::Result<()> {
    let b64_table = lua.create_table()?;

    let encode_fn = lua.create_function(|_, input: String| Ok(BASE64.encode(input.as_bytes())))?;
    b64_table.set("encode", encode_fn)?;

    let decode_fn = lua.create_function(|_, input: String| {
        let bytes = BASE64
            .decode(input.as_bytes())
            .map_err(|e| mlua::Error::runtime(format!("base64.decode: {e}")))?;
        String::from_utf8(bytes)
            .map_err(|e| mlua::Error::runtime(format!("base64.decode: invalid UTF-8: {e}")))
    })?;
    b64_table.set("decode", decode_fn)?;

    lua.globals().set("base64", b64_table)?;
    Ok(())
}

fn register_crypto(lua: &Lua) -> mlua::Result<()> {
    let crypto_table = lua.create_table()?;

    let jwt_sign_fn = lua.create_function(|_, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();

        let claims_table = match args_iter.next() {
            Some(Value::Table(t)) => t,
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.jwt_sign: first argument must be a claims table",
                ));
            }
        };

        let pem_key: String = match args_iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.jwt_sign: second argument must be a PEM key string",
                ));
            }
        };

        let algorithm = match args_iter.next() {
            Some(Value::String(s)) => match s.to_str()?.to_uppercase().as_str() {
                "RS256" => Algorithm::RS256,
                "RS384" => Algorithm::RS384,
                "RS512" => Algorithm::RS512,
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "crypto.jwt_sign: unsupported algorithm: {other}"
                    )));
                }
            },
            Some(Value::Nil) | None => Algorithm::RS256,
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.jwt_sign: third argument must be an algorithm string or nil",
                ));
            }
        };

        let claims_json = lua_value_to_json(&Value::Table(claims_table))?;
        let pem_bytes = Zeroizing::new(pem_key.into_bytes());
        let key = EncodingKey::from_rsa_pem(&pem_bytes)
            .map_err(|e| mlua::Error::runtime(format!("crypto.jwt_sign: invalid PEM key: {e}")))?;

        let header = Header::new(algorithm);
        let token = jsonwebtoken::encode(&header, &claims_json, &key)
            .map_err(|e| mlua::Error::runtime(format!("crypto.jwt_sign: encoding failed: {e}")))?;

        Ok(token)
    })?;
    crypto_table.set("jwt_sign", jwt_sign_fn)?;

    let hash_fn = lua.create_function(|_, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();

        let input: String = match args_iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.hash: first argument must be a string",
                ));
            }
        };

        let algorithm: String = match args_iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_lowercase(),
            Some(Value::Nil) | None => "sha256".to_string(),
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.hash: second argument must be an algorithm string or nil",
                ));
            }
        };

        let hex = match algorithm.as_str() {
            "sha224" => format!("{:x}", sha2::Sha224::digest(input.as_bytes())),
            "sha256" => format!("{:x}", sha2::Sha256::digest(input.as_bytes())),
            "sha384" => format!("{:x}", sha2::Sha384::digest(input.as_bytes())),
            "sha512" => format!("{:x}", sha2::Sha512::digest(input.as_bytes())),
            "sha3-224" => format!("{:x}", sha3::Sha3_224::digest(input.as_bytes())),
            "sha3-256" => format!("{:x}", sha3::Sha3_256::digest(input.as_bytes())),
            "sha3-384" => format!("{:x}", sha3::Sha3_384::digest(input.as_bytes())),
            "sha3-512" => format!("{:x}", sha3::Sha3_512::digest(input.as_bytes())),
            other => {
                return Err(mlua::Error::runtime(format!(
                    "crypto.hash: unsupported algorithm: {other} (supported: sha224, sha256, sha384, sha512, sha3-224, sha3-256, sha3-384, sha3-512)"
                )));
            }
        };

        Ok(hex)
    })?;
    crypto_table.set("hash", hash_fn)?;

    let random_fn = lua.create_function(|_, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();

        let length: usize = match args_iter.next() {
            Some(Value::Integer(n)) if n > 0 => n as usize,
            Some(Value::Integer(n)) => {
                return Err(mlua::Error::runtime(format!(
                    "crypto.random: length must be positive, got {n}"
                )));
            }
            Some(Value::Nil) | None => 32,
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.random: first argument must be a positive integer or nil",
                ));
            }
        };

        let charset: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let mut rng = rand::rng();
        let result: String = (0..length)
            .map(|_| charset[rng.random_range(..charset.len())] as char)
            .collect();

        Ok(result)
    })?;
    crypto_table.set("random", random_fn)?;

    lua.globals().set("crypto", crypto_table)?;
    Ok(())
}

fn register_regex(lua: &Lua) -> mlua::Result<()> {
    let regex_table = lua.create_table()?;

    let match_fn = lua.create_function(|_, (text, pattern): (String, String)| {
        let re = regex_lite::Regex::new(&pattern)
            .map_err(|e| mlua::Error::runtime(format!("regex.match: invalid pattern: {e}")))?;
        Ok(re.is_match(&text))
    })?;
    regex_table.set("match", match_fn)?;

    let find_fn = lua.create_function(|lua, (text, pattern): (String, String)| {
        let re = regex_lite::Regex::new(&pattern)
            .map_err(|e| mlua::Error::runtime(format!("regex.find: invalid pattern: {e}")))?;
        match re.captures(&text) {
            Some(caps) => {
                let result = lua.create_table()?;
                let full_match = caps.get(0).map(|m| m.as_str()).unwrap_or("");
                result.set("match", full_match.to_string())?;
                let groups = lua.create_table()?;
                for i in 1..caps.len() {
                    if let Some(m) = caps.get(i) {
                        groups.set(i, m.as_str().to_string())?;
                    }
                }
                result.set("groups", groups)?;
                Ok(Value::Table(result))
            }
            None => Ok(Value::Nil),
        }
    })?;
    regex_table.set("find", find_fn)?;

    let find_all_fn = lua.create_function(|lua, (text, pattern): (String, String)| {
        let re = regex_lite::Regex::new(&pattern)
            .map_err(|e| mlua::Error::runtime(format!("regex.find_all: invalid pattern: {e}")))?;
        let results = lua.create_table()?;
        for (i, m) in re.find_iter(&text).enumerate() {
            results.set(i + 1, m.as_str().to_string())?;
        }
        Ok(results)
    })?;
    regex_table.set("find_all", find_all_fn)?;

    let replace_fn =
        lua.create_function(|_, (text, pattern, replacement): (String, String, String)| {
            let re = regex_lite::Regex::new(&pattern).map_err(|e| {
                mlua::Error::runtime(format!("regex.replace: invalid pattern: {e}"))
            })?;
            Ok(re.replace_all(&text, replacement.as_str()).into_owned())
        })?;
    regex_table.set("replace", replace_fn)?;

    lua.globals().set("regex", regex_table)?;
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
    fn test_base64_roundtrip() {
        let input = "hello world";
        let encoded = BASE64.encode(input.as_bytes());
        assert_eq!(encoded, "aGVsbG8gd29ybGQ=");
        let decoded = BASE64.decode(encoded.as_bytes()).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), input);
    }

    #[test]
    fn test_base64_empty() {
        let encoded = BASE64.encode(b"");
        assert_eq!(encoded, "");
        let decoded = BASE64.decode(b"").unwrap();
        assert!(decoded.is_empty());
    }

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
