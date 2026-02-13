use data_encoding::BASE64;
use mlua::{Lua, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

pub fn register_log(lua: &Lua) -> mlua::Result<()> {
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

pub fn register_env(lua: &Lua) -> mlua::Result<()> {
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

pub fn register_sleep(lua: &Lua) -> mlua::Result<()> {
    let sleep_fn = lua.create_async_function(|_, seconds: f64| async move {
        let duration = std::time::Duration::from_secs_f64(seconds);
        tokio::time::sleep(duration).await;
        Ok(())
    })?;
    lua.globals().set("sleep", sleep_fn)?;
    Ok(())
}

pub fn register_time(lua: &Lua) -> mlua::Result<()> {
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

pub fn register_fs(lua: &Lua) -> mlua::Result<()> {
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

pub fn register_base64(lua: &Lua) -> mlua::Result<()> {
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

pub fn register_regex(lua: &Lua) -> mlua::Result<()> {
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

pub fn register_async(lua: &Lua) -> mlua::Result<()> {
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

#[cfg(test)]
mod tests {
    use data_encoding::BASE64;

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
}
