use data_encoding::BASE64;
use digest::Digest;
use futures_util::{SinkExt, StreamExt};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use mlua::{Lua, Table, UserData, Value};
use rand::RngExt;
use sqlx::any::AnyRow;
use sqlx::{AnyPool, Column, Row, ValueRef};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tokio_tungstenite::MaybeTlsStream;
use tracing::{error, info, warn};
use zeroize::Zeroizing;

pub fn register_all(lua: &Lua, client: reqwest::Client) -> mlua::Result<()> {
    register_http(lua, client)?;
    register_json(lua)?;
    register_yaml(lua)?;
    register_toml(lua)?;
    register_assert(lua)?;
    register_log(lua)?;
    register_env(lua)?;
    register_sleep(lua)?;
    register_time(lua)?;
    register_fs(lua)?;
    register_base64(lua)?;
    register_crypto(lua)?;
    register_regex(lua)?;
    register_async(lua)?;
    register_db(lua)?;
    register_ws(lua)?;
    register_template(lua)?;
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

    let serve_fn =
        lua.create_async_function(|lua, args: mlua::MultiValue| async move {
            let mut args_iter = args.into_iter();

            let port: u16 = match args_iter.next() {
                Some(Value::Integer(n)) => n as u16,
                _ => {
                    return Err::<(), _>(mlua::Error::runtime(
                        "http.serve: first argument must be a port number",
                    ));
                }
            };

            let routes_table = match args_iter.next() {
                Some(Value::Table(t)) => t,
                _ => {
                    return Err::<(), _>(mlua::Error::runtime(
                        "http.serve: second argument must be a routes table",
                    ));
                }
            };

            let routes = Rc::new(parse_routes(&routes_table)?);

            let listener = TcpListener::bind(format!("0.0.0.0:{port}"))
                .await
                .map_err(|e| mlua::Error::runtime(format!("http.serve: bind failed: {e}")))?;

            loop {
                let (stream, _addr) = listener.accept().await.map_err(|e| {
                    mlua::Error::runtime(format!("http.serve: accept failed: {e}"))
                })?;

                let routes = routes.clone();
                let lua_clone = lua.clone();

                tokio::task::spawn_local(async move {
                    let io = hyper_util::rt::TokioIo::new(stream);
                    let routes = routes.clone();
                    let lua = lua_clone.clone();

                    let service = service_fn(move |req: Request<Incoming>| {
                        let routes = routes.clone();
                        let lua = lua.clone();
                        async move { handle_request(&lua, &routes, req).await }
                    });

                    if let Err(e) = http1::Builder::new().serve_connection(io, service).await
                        && !e.to_string().contains("connection closed")
                    {
                        error!("http.serve: connection error: {e}");
                    }
                });
            }
        })?;
    http_table.set("serve", serve_fn)?;

    lua.globals().set("http", http_table)?;
    Ok(())
}

fn parse_routes(routes_table: &Table) -> mlua::Result<HashMap<(String, String), mlua::Function>> {
    let mut routes = HashMap::new();
    for method_pair in routes_table.pairs::<String, Table>() {
        let (method, paths_table) = method_pair?;
        let method_upper = method.to_uppercase();
        for path_pair in paths_table.pairs::<String, mlua::Function>() {
            let (path, func) = path_pair?;
            routes.insert((method_upper.clone(), path), func);
        }
    }
    Ok(routes)
}

async fn handle_request(
    lua: &Lua,
    routes: &HashMap<(String, String), mlua::Function>,
    req: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
        .collect();

    let body_bytes = match http_body_util::BodyExt::collect(req.into_body()).await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => Bytes::new(),
    };
    let body_str = String::from_utf8_lossy(&body_bytes).to_string();

    let key = (method.clone(), path.clone());
    let handler = match routes.get(&key) {
        Some(f) => f,
        None => {
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("content-type", "text/plain")
                .body(Full::new(Bytes::from("not found")))
                .unwrap());
        }
    };

    match build_lua_request_and_call(lua, handler, &method, &path, &query, &headers, &body_str) {
        Ok(lua_resp) => lua_response_to_http(&lua_resp),
        Err(e) => Ok(Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header("content-type", "text/plain")
            .body(Full::new(Bytes::from(format!("handler error: {e}"))))
            .unwrap()),
    }
}

fn build_lua_request_and_call(
    lua: &Lua,
    handler: &mlua::Function,
    method: &str,
    path: &str,
    query: &str,
    headers: &[(String, String)],
    body: &str,
) -> mlua::Result<Table> {
    let req_table = lua.create_table()?;
    req_table.set("method", method.to_string())?;
    req_table.set("path", path.to_string())?;
    req_table.set("query", query.to_string())?;
    req_table.set("body", body.to_string())?;

    let headers_table = lua.create_table()?;
    for (k, v) in headers {
        headers_table.set(k.as_str(), v.as_str())?;
    }
    req_table.set("headers", headers_table)?;

    handler.call::<Table>(req_table)
}

fn lua_response_to_http(
    resp_table: &Table,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let status = resp_table
        .get::<Option<u16>>("status")
        .unwrap_or(None)
        .unwrap_or(200);

    let mut builder =
        Response::builder().status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK));

    if let Ok(Some(headers_table)) = resp_table.get::<Option<Table>>("headers") {
        for (k, v) in headers_table.pairs::<String, String>().flatten() {
            builder = builder.header(k, v);
        }
    }

    let body_bytes = if let Ok(Some(json_table)) = resp_table.get::<Option<Table>>("json") {
        let json_val =
            lua_value_to_json(&Value::Table(json_table)).unwrap_or(serde_json::Value::Null);
        let serialized = serde_json::to_string(&json_val).unwrap_or_else(|_| "null".to_string());
        builder = builder.header("content-type", "application/json");
        Bytes::from(serialized)
    } else if let Ok(Some(body_str)) = resp_table.get::<Option<String>>("body") {
        builder = builder.header("content-type", "text/plain");
        Bytes::from(body_str)
    } else {
        builder = builder.header("content-type", "text/plain");
        Bytes::new()
    };

    Ok(builder.body(Full::new(body_bytes)).unwrap())
}

fn register_json(lua: &Lua) -> mlua::Result<()> {
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

    let write_fn = lua.create_function(|_, (path, content): (String, String)| {
        let p = std::path::Path::new(&path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                mlua::Error::runtime(format!(
                    "fs.write: failed to create directories for {path:?}: {e}"
                ))
            })?;
        }
        std::fs::write(&path, &content)
            .map_err(|e| mlua::Error::runtime(format!("fs.write: failed to write {path:?}: {e}")))
    })?;
    fs_table.set("write", write_fn)?;

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

        // 4th optional argument: options table with header fields (kid, typ, etc.)
        let opts = match args_iter.next() {
            Some(Value::Table(t)) => Some(t),
            Some(Value::Nil) | None => None,
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.jwt_sign: fourth argument must be an options table or nil",
                ));
            }
        };

        let claims_json = lua_value_to_json(&Value::Table(claims_table))?;
        let pem_bytes = Zeroizing::new(pem_key.into_bytes());
        let key = EncodingKey::from_rsa_pem(&pem_bytes)
            .map_err(|e| mlua::Error::runtime(format!("crypto.jwt_sign: invalid PEM key: {e}")))?;

        let mut header = Header::new(algorithm);
        if let Some(ref opts_table) = opts {
            if let Ok(kid) = opts_table.get::<String>("kid") {
                header.kid = Some(kid);
            }
        }
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

fn register_yaml(lua: &Lua) -> mlua::Result<()> {
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

fn register_toml(lua: &Lua) -> mlua::Result<()> {
    let toml_table = lua.create_table()?;

    let parse_fn = lua.create_function(|lua, s: String| {
        let toml_val: toml::Value = toml::from_str(&s)
            .map_err(|e| mlua::Error::runtime(format!("toml.parse: {e}")))?;
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

    let replace_fn = lua.create_function(
        |_, (text, pattern, replacement): (String, String, String)| {
            let re = regex_lite::Regex::new(&pattern).map_err(|e| {
                mlua::Error::runtime(format!("regex.replace: invalid pattern: {e}"))
            })?;
            Ok(re.replace_all(&text, replacement.as_str()).into_owned())
        },
    )?;
    regex_table.set("replace", replace_fn)?;

    lua.globals().set("regex", regex_table)?;
    Ok(())
}

fn register_async(lua: &Lua) -> mlua::Result<()> {
    let async_table = lua.create_table()?;

    let spawn_fn = lua.create_async_function(|lua, func: mlua::Function| async move {
        let thread = lua.create_thread(func)?;
        let async_thread = thread.into_async::<mlua::MultiValue>(())?;
        let join_handle: tokio::task::JoinHandle<Result<Vec<Value>, String>> =
            tokio::task::spawn_local(async move {
                let values = async_thread.await.map_err(|e| e.to_string())?;
                Ok(values.into_vec())
            });

        let handle = lua.create_table()?;
        let cell = std::rc::Rc::new(std::cell::RefCell::new(Some(join_handle)));
        let cell_clone = cell.clone();

        let await_fn = lua.create_async_function(move |lua, ()| {
            let cell = cell_clone.clone();
            async move {
                let join_handle = cell
                    .borrow_mut()
                    .take()
                    .ok_or_else(|| mlua::Error::runtime("async handle already awaited"))?;
                let result = join_handle.await.map_err(|e| {
                    mlua::Error::runtime(format!("async.spawn: task panicked: {e}"))
                })?;
                match result {
                    Ok(values) => {
                        let tbl = lua.create_table()?;
                        for (i, v) in values.into_iter().enumerate() {
                            tbl.set(i + 1, v)?;
                        }
                        Ok(Value::Table(tbl))
                    }
                    Err(msg) => Err(mlua::Error::runtime(msg)),
                }
            }
        })?;
        handle.set("await", await_fn)?;

        Ok(handle)
    })?;
    async_table.set("spawn", spawn_fn)?;

    let spawn_interval_fn =
        lua.create_async_function(|lua, (seconds, func): (f64, mlua::Function)| async move {
            if seconds <= 0.0 {
                return Err(mlua::Error::runtime(
                    "async.spawn_interval: interval must be positive",
                ));
            }

            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let cancel_clone = cancel.clone();

            tokio::task::spawn_local({
                let cancel = cancel_clone.clone();
                async move {
                    let mut interval =
                        tokio::time::interval(std::time::Duration::from_secs_f64(seconds));
                    interval.tick().await;
                    loop {
                        interval.tick().await;
                        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                            break;
                        }
                        if let Err(e) = func.call_async::<()>(()).await {
                            error!("async.spawn_interval: callback error: {e}");
                            break;
                        }
                    }
                }
            });

            let handle = lua.create_table()?;
            let cancel_fn = lua.create_function(move |_, ()| {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            })?;
            handle.set("cancel", cancel_fn)?;

            Ok(handle)
        })?;
    async_table.set("spawn_interval", spawn_interval_fn)?;

    lua.globals().set("async", async_table)?;
    Ok(())
}

struct DbPool(Arc<AnyPool>);
impl UserData for DbPool {}

fn register_db(lua: &Lua) -> mlua::Result<()> {
    sqlx::any::install_default_drivers();

    let db_table = lua.create_table()?;

    let connect_fn = lua.create_async_function(|lua, url: String| async move {
        let pool = sqlx::any::AnyPoolOptions::new()
            .max_connections(if url.starts_with("sqlite:") { 1 } else { 5 })
            .connect(&url)
            .await
            .map_err(|e| mlua::Error::runtime(format!("db.connect: {e}")))?;
        lua.create_any_userdata(DbPool(Arc::new(pool)))
    })?;
    db_table.set("connect", connect_fn)?;

    let query_fn =
        lua.create_async_function(|lua, args: mlua::MultiValue| async move {
            let mut args_iter = args.into_iter();

            let pool = extract_db_pool(&args_iter.next(), "db.query")?;
            let sql = extract_sql_string(&args_iter.next(), "db.query")?;
            let params = extract_params(&args_iter.next())?;

            let mut query = sqlx::query(&sql);
            for p in &params {
                query = bind_param(query, p);
            }

            let rows: Vec<AnyRow> = query
                .fetch_all(&*pool)
                .await
                .map_err(|e| mlua::Error::runtime(format!("db.query: {e}")))?;

            let result = lua.create_table()?;
            for (i, row) in rows.iter().enumerate() {
                let row_table = any_row_to_lua_table(&lua, row)?;
                result.set(i + 1, row_table)?;
            }
            Ok(Value::Table(result))
        })?;
    db_table.set("query", query_fn)?;

    let execute_fn =
        lua.create_async_function(|lua, args: mlua::MultiValue| async move {
            let mut args_iter = args.into_iter();

            let pool = extract_db_pool(&args_iter.next(), "db.execute")?;
            let sql = extract_sql_string(&args_iter.next(), "db.execute")?;
            let params = extract_params(&args_iter.next())?;

            let mut query = sqlx::query(&sql);
            for p in &params {
                query = bind_param(query, p);
            }

            let result = query
                .execute(&*pool)
                .await
                .map_err(|e| mlua::Error::runtime(format!("db.execute: {e}")))?;

            let tbl = lua.create_table()?;
            tbl.set("rows_affected", result.rows_affected() as i64)?;
            Ok(Value::Table(tbl))
        })?;
    db_table.set("execute", execute_fn)?;

    let close_fn = lua.create_async_function(|_, args: mlua::MultiValue| async move {
        let mut args_iter = args.into_iter();
        let pool = extract_db_pool(&args_iter.next(), "db.close")?;
        pool.close().await;
        Ok(())
    })?;
    db_table.set("close", close_fn)?;

    lua.globals().set("db", db_table)?;
    Ok(())
}

fn extract_db_pool(val: &Option<Value>, fn_name: &str) -> mlua::Result<Arc<AnyPool>> {
    match val {
        Some(Value::UserData(ud)) => {
            let db = ud
                .borrow::<DbPool>()
                .map_err(|_| mlua::Error::runtime(format!("{fn_name}: first argument must be a db connection")))?;
            Ok(db.0.clone())
        }
        _ => Err(mlua::Error::runtime(format!(
            "{fn_name}: first argument must be a db connection"
        ))),
    }
}

fn extract_sql_string(val: &Option<Value>, fn_name: &str) -> mlua::Result<String> {
    match val {
        Some(Value::String(s)) => Ok(s.to_str()?.to_string()),
        _ => Err(mlua::Error::runtime(format!(
            "{fn_name}: second argument must be a SQL string"
        ))),
    }
}

#[derive(Clone)]
enum DbParam {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

fn extract_params(val: &Option<Value>) -> mlua::Result<Vec<DbParam>> {
    match val {
        Some(Value::Table(t)) => {
            let mut params = Vec::new();
            let len = t.len()?;
            for i in 1..=len {
                let v: Value = t.get(i)?;
                let param = match v {
                    Value::Nil => DbParam::Null,
                    Value::Boolean(b) => DbParam::Bool(b),
                    Value::Integer(n) => DbParam::Int(n),
                    Value::Number(f) => DbParam::Float(f),
                    Value::String(s) => DbParam::Text(s.to_str()?.to_string()),
                    _ => {
                        return Err(mlua::Error::runtime(format!(
                            "db: unsupported parameter type: {}",
                            v.type_name()
                        )));
                    }
                };
                params.push(param);
            }
            Ok(params)
        }
        Some(Value::Nil) | None => Ok(Vec::new()),
        _ => Err(mlua::Error::runtime(
            "db: params must be a table (array) or nil",
        )),
    }
}

fn bind_param<'q>(
    query: sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>>,
    param: &'q DbParam,
) -> sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>> {
    match param {
        DbParam::Null => query.bind(None::<String>),
        DbParam::Bool(b) => query.bind(*b),
        DbParam::Int(n) => query.bind(*n),
        DbParam::Float(f) => query.bind(*f),
        DbParam::Text(s) => query.bind(s.as_str()),
    }
}

fn any_row_to_lua_table(lua: &Lua, row: &AnyRow) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for col in row.columns() {
        let name = col.name();
        let val: Value = any_column_to_lua_value(lua, row, col)?;
        table.set(name.to_string(), val)?;
    }
    Ok(table)
}

fn any_column_to_lua_value<C: Column>(
    lua: &Lua,
    row: &AnyRow,
    col: &C,
) -> mlua::Result<Value> {
    let ordinal = col.ordinal();
    let type_info = col.type_info();
    let type_name = type_info.to_string();
    let type_name = type_name.to_uppercase();

    if row.try_get_raw(ordinal).map(|v| v.is_null()).unwrap_or(true) {
        return Ok(Value::Nil);
    }

    match type_name.as_str() {
        "BOOLEAN" | "BOOL" => {
            let v: bool = row
                .try_get(ordinal)
                .map_err(|e| mlua::Error::runtime(format!("db: column read error: {e}")))?;
            Ok(Value::Boolean(v))
        }
        "INTEGER" | "INT" | "INT4" | "INT8" | "BIGINT" | "SMALLINT" | "TINYINT" | "INT2" => {
            let v: i64 = row
                .try_get(ordinal)
                .map_err(|e| mlua::Error::runtime(format!("db: column read error: {e}")))?;
            Ok(Value::Integer(v))
        }
        "REAL" | "FLOAT" | "FLOAT4" | "FLOAT8" | "DOUBLE" | "DOUBLE PRECISION" | "NUMERIC" => {
            let v: f64 = row
                .try_get(ordinal)
                .map_err(|e| mlua::Error::runtime(format!("db: column read error: {e}")))?;
            Ok(Value::Number(v))
        }
        _ => {
            let v: String = row
                .try_get(ordinal)
                .map_err(|e| mlua::Error::runtime(format!("db: column read error: {e}")))?;
            Ok(Value::String(lua.create_string(&v)?))
        }
    }
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

type WsSink = Rc<
    tokio::sync::Mutex<
        futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
            tokio_tungstenite::tungstenite::Message,
        >,
    >,
>;
type WsStream = Rc<
    tokio::sync::Mutex<
        futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
        >,
    >,
>;

struct WsConn {
    sink: WsSink,
    stream: WsStream,
}
impl UserData for WsConn {}

fn extract_ws_conn(val: &Value, fn_name: &str) -> mlua::Result<(WsSink, WsStream)> {
    let ud = match val {
        Value::UserData(ud) => ud,
        _ => {
            return Err(mlua::Error::runtime(format!(
                "{fn_name}: first argument must be a ws connection"
            )));
        }
    };
    let ws = ud.borrow::<WsConn>().map_err(|_| {
        mlua::Error::runtime(format!(
            "{fn_name}: first argument must be a ws connection"
        ))
    })?;
    Ok((ws.sink.clone(), ws.stream.clone()))
}

fn register_ws(lua: &Lua) -> mlua::Result<()> {
    let ws_table = lua.create_table()?;

    let connect_fn = lua.create_async_function(|lua, url: String| async move {
        let (stream, _response) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|e| mlua::Error::runtime(format!("ws.connect: {e}")))?;
        let (sink, read) = stream.split();
        lua.create_any_userdata(WsConn {
            sink: Rc::new(tokio::sync::Mutex::new(sink)),
            stream: Rc::new(tokio::sync::Mutex::new(read)),
        })
    })?;
    ws_table.set("connect", connect_fn)?;

    let send_fn = lua.create_async_function(|_, (conn, msg): (Value, String)| async move {
        let (sink, _stream) = extract_ws_conn(&conn, "ws.send")?;
        sink.lock()
            .await
            .send(tokio_tungstenite::tungstenite::Message::Text(msg.into()))
            .await
            .map_err(|e| mlua::Error::runtime(format!("ws.send: {e}")))?;
        Ok(())
    })?;
    ws_table.set("send", send_fn)?;

    let recv_fn = lua.create_async_function(|_, conn: Value| async move {
        let (_sink, stream) = extract_ws_conn(&conn, "ws.recv")?;
        loop {
            let msg = stream
                .lock()
                .await
                .next()
                .await
                .ok_or_else(|| mlua::Error::runtime("ws.recv: connection closed"))?
                .map_err(|e| mlua::Error::runtime(format!("ws.recv: {e}")))?;
            match msg {
                tokio_tungstenite::tungstenite::Message::Text(t) => {
                    return Ok(t.to_string());
                }
                tokio_tungstenite::tungstenite::Message::Binary(b) => {
                    return String::from_utf8(b.into()).map_err(|e| {
                        mlua::Error::runtime(format!("ws.recv: invalid UTF-8: {e}"))
                    });
                }
                tokio_tungstenite::tungstenite::Message::Close(_) => {
                    return Err(mlua::Error::runtime("ws.recv: connection closed"));
                }
                _ => continue,
            }
        }
    })?;
    ws_table.set("recv", recv_fn)?;

    let close_fn = lua.create_async_function(|_, conn: Value| async move {
        let (sink, _stream) = extract_ws_conn(&conn, "ws.close")?;
        sink.lock()
            .await
            .close()
            .await
            .map_err(|e| mlua::Error::runtime(format!("ws.close: {e}")))?;
        Ok(())
    })?;
    ws_table.set("close", close_fn)?;

    lua.globals().set("ws", ws_table)?;
    Ok(())
}

fn register_template(lua: &Lua) -> mlua::Result<()> {
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
        let env = minijinja::Environment::new();
        let tmpl = env
            .template_from_str(&template_str)
            .map_err(|e| mlua::Error::runtime(format!("template.render_string: {e}")))?;
        tmpl.render(mini_vars)
            .map_err(|e| mlua::Error::runtime(format!("template.render_string: {e}")))
    })?;
    tmpl_table.set("render_string", render_string_fn)?;

    let render_fn = lua.create_function(|_, (file_path, vars): (String, Value)| {
        let content = std::fs::read_to_string(&file_path).map_err(|e| {
            mlua::Error::runtime(format!("template.render: failed to read {file_path:?}: {e}"))
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

    lua.globals().set("template", tmpl_table)?;
    Ok(())
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
