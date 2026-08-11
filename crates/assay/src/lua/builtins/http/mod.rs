use super::json::lua_table_to_json;
use mlua::{Lua, Table, UserData, Value};
use rand::RngExt;
#[cfg(feature = "server")]
mod server;
#[cfg(feature = "server")]
pub use server::LuaAxumRouter;

struct HttpClient(reqwest::Client);
impl UserData for HttpClient {}

/// Registers `http.client(opts)` and the `http._client_request` shim that
/// dispatches a call made on one of those client handles.
fn register_client_handles(lua: &Lua, http_table: &Table) -> mlua::Result<()> {
    let client_fn = lua.create_async_function(|lua, opts: Option<Table>| async move {
        let mut builder = reqwest::Client::builder();

        let timeout_secs: f64 = opts
            .as_ref()
            .and_then(|t| t.get::<f64>("timeout").ok())
            .unwrap_or(30.0);
        builder = builder.timeout(std::time::Duration::from_secs_f64(timeout_secs));

        let follow_redirects: bool = opts
            .as_ref()
            .and_then(|t| t.get::<bool>("follow_redirects").ok())
            .unwrap_or(true);
        if !follow_redirects {
            builder = builder.redirect(reqwest::redirect::Policy::none());
        }

        if let Some(ref opts_table) = opts {
            if let Ok(ca_path) = opts_table.get::<String>("ca_cert_file") {
                let pem = std::fs::read(&ca_path).map_err(|e| {
                    mlua::Error::runtime(format!(
                        "http.client: failed to read CA cert file {ca_path:?}: {e}"
                    ))
                })?;
                let cert = reqwest::Certificate::from_pem(&pem).map_err(|e| {
                    mlua::Error::runtime(format!("http.client: invalid PEM in {ca_path:?}: {e}"))
                })?;
                builder = builder.add_root_certificate(cert);
            }
            if let Ok(ca_pem) = opts_table.get::<String>("ca_cert") {
                let cert = reqwest::Certificate::from_pem(ca_pem.as_bytes()).map_err(|e| {
                    mlua::Error::runtime(format!("http.client: invalid CA cert PEM: {e}"))
                })?;
                builder = builder.add_root_certificate(cert);
            }
        }

        let client = builder.build().map_err(|e| {
            mlua::Error::runtime(format!("http.client: failed to build client: {e}"))
        })?;

        let ud = lua.create_any_userdata(HttpClient(client))?;

        let wrapper: Table = lua
            .load(
                r#"
                local ud = ...
                local obj = { _ud = ud }
                setmetatable(obj, {
                    __index = {
                        get = function(self, url, opts)
                            return http._client_request(self._ud, "get", url, opts)
                        end,
                        post = function(self, url, body, opts)
                            return http._client_request(self._ud, "post", url, body, opts)
                        end,
                        put = function(self, url, body, opts)
                            return http._client_request(self._ud, "put", url, body, opts)
                        end,
                        patch = function(self, url, body, opts)
                            return http._client_request(self._ud, "patch", url, body, opts)
                        end,
                        delete = function(self, url, opts)
                            return http._client_request(self._ud, "delete", url, opts)
                        end,
                    }
                })
                return obj
            "#,
            )
            .call(ud)?;

        Ok(Value::Table(wrapper))
    })?;
    http_table.set("client", client_fn)?;

    let client_request_fn =
        lua.create_async_function(|lua, args: mlua::MultiValue| async move {
            let mut args_iter = args.into_iter();

            let client = match args_iter.next() {
                Some(Value::UserData(ud)) => {
                    let hc = ud.borrow::<HttpClient>().map_err(|_| {
                        mlua::Error::runtime(
                            "http._client_request: first arg must be an http client",
                        )
                    })?;
                    hc.0.clone()
                }
                _ => {
                    return Err(mlua::Error::runtime(
                        "http._client_request: first arg must be an http client",
                    ));
                }
            };

            let method_name: String = match args_iter.next() {
                Some(Value::String(s)) => s.to_str()?.to_string(),
                _ => {
                    return Err(mlua::Error::runtime(
                        "http._client_request: second arg must be method name",
                    ));
                }
            };

            let remaining: mlua::MultiValue = args_iter.collect();
            execute_http_request(&lua, &client, &method_name, remaining).await
        })?;
    http_table.set("_client_request", client_request_fn)?;
    Ok(())
}

/// Registers `http.download(url, path, opts?)`.
fn register_download(lua: &Lua, client: reqwest::Client, http_table: &Table) -> mlua::Result<()> {
    let download_fn = lua.create_async_function(move |_, args: mlua::MultiValue| {
        let client = client.clone();
        async move {
            use futures_util::StreamExt;
            use tokio::io::AsyncWriteExt;

            let mut args_iter = args.into_iter();
            let url: String = match args_iter.next() {
                Some(mlua::Value::String(s)) => s.to_str()?.to_string(),
                _ => {
                    return Err(mlua::Error::runtime(
                        "http.download: first arg must be url string",
                    ));
                }
            };
            let path: String = match args_iter.next() {
                Some(mlua::Value::String(s)) => s.to_str()?.to_string(),
                _ => {
                    return Err(mlua::Error::runtime(
                        "http.download: second arg must be dest path string",
                    ));
                }
            };
            // Optional opts table: { headers = {...}, timeout = secs }
            let opts: Option<mlua::Table> = match args_iter.next() {
                Some(mlua::Value::Table(t)) => Some(t),
                _ => None,
            };

            // Build request
            let mut req = client.get(&url);
            if let Some(ref t) = opts {
                if let Ok(h) = t.get::<mlua::Table>("headers") {
                    for pair in h.pairs::<String, String>() {
                        let (k, v) = pair?;
                        req = req.header(&k, &v);
                    }
                }
                if let Ok(secs) = t.get::<f64>("timeout")
                    && secs.is_finite()
                    && secs > 0.0
                {
                    req = req.timeout(std::time::Duration::from_secs_f64(secs));
                }
            }

            // Optional max_size cap. Defaults to 1 GiB so a malicious URL
            // can't fill the disk. Caller can pass max_size = 0 to disable.
            const DEFAULT_MAX_SIZE: i64 = 1024 * 1024 * 1024;
            let max_size: i64 = opts
                .as_ref()
                .and_then(|t| t.get::<i64>("max_size").ok())
                .unwrap_or(DEFAULT_MAX_SIZE);

            // Ensure parent dir
            if let Some(parent) = std::path::Path::new(&path).parent()
                && !parent.as_os_str().is_empty()
            {
                tokio::fs::create_dir_all(parent).await.map_err(|e| {
                    mlua::Error::runtime(format!("http.download: mkdir parent: {e}"))
                })?;
            }

            // Open temp file at <path>.tmp.<random>. Random suffix instead of
            // PID — a co-located unprivileged process can pre-create symlinks
            // at predictable PID-based paths.
            let tmp = format!("{path}.tmp.{:016x}", rand::rng().random::<u64>());
            let mut file = tokio::fs::File::create(&tmp).await.map_err(|e| {
                mlua::Error::runtime(format!("http.download: create temp {tmp:?}: {e}"))
            })?;

            // Cleanup helper closure result
            let do_download = async {
                let resp = req
                    .send()
                    .await
                    .map_err(|e| mlua::Error::runtime(format!("http.download: request: {e}")))?;
                if !resp.status().is_success() {
                    return Err(mlua::Error::runtime(format!(
                        "http.download: HTTP {} for {url}",
                        resp.status()
                    )));
                }
                let mut total: i64 = 0;
                let mut stream = resp.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    let bytes = chunk
                        .map_err(|e| mlua::Error::runtime(format!("http.download: stream: {e}")))?;
                    total += bytes.len() as i64;
                    if max_size > 0 && total > max_size {
                        return Err(mlua::Error::runtime(format!(
                            "http.download: response exceeds max_size ({total} > {max_size} bytes) for {url}"
                        )));
                    }
                    file.write_all(&bytes)
                        .await
                        .map_err(|e| mlua::Error::runtime(format!("http.download: write: {e}")))?;
                }
                file.flush()
                    .await
                    .map_err(|e| mlua::Error::runtime(format!("http.download: flush: {e}")))?;
                file.sync_all()
                    .await
                    .map_err(|e| mlua::Error::runtime(format!("http.download: fsync: {e}")))?;
                drop(file); // close before rename on Windows; harmless on Linux
                Ok(total)
            };

            match do_download.await {
                Ok(total) => {
                    tokio::fs::rename(&tmp, &path).await.map_err(|e| {
                        mlua::Error::runtime(format!(
                            "http.download: rename {tmp:?} -> {path:?}: {e}"
                        ))
                    })?;
                    Ok(total)
                }
                Err(e) => {
                    let _ = tokio::fs::remove_file(&tmp).await;
                    Err(e)
                }
            }
        }
    })?;
    http_table.set("download", download_fn)?;
    Ok(())
}

pub fn register_http(lua: &Lua, client: reqwest::Client) -> mlua::Result<()> {
    let http_table = lua.create_table()?;

    for method in ["get", "post", "put", "patch", "delete"] {
        let method_client = client.clone();
        let method_name = method.to_string();

        let func = lua.create_async_function(move |lua, args: mlua::MultiValue| {
            let client = method_client.clone();
            let method_name = method_name.clone();
            async move { execute_http_request(&lua, &client, &method_name, args).await }
        })?;
        http_table.set(method, func)?;
    }

    register_client_handles(lua, &http_table)?;

    #[cfg(feature = "server")]
    server::register_serve(lua, &http_table)?;

    // http.download(url, path, opts?) -> bytes_written
    // Streams the response body to disk via a temp file, then atomic-renames into place.
    // Creates parent directories as needed. On any failure (4xx/5xx, IO error, network),
    // the temp file is removed and the error propagates — no partial file at `path`.
    register_download(lua, client, &http_table)?;

    lua.globals().set("http", http_table)?;
    Ok(())
}

/// Parses the Lua call shape into `(url, body, auto_json, opts)`.
///
/// `get`/`delete` have no body slot in their shorthand, so a body for those
/// arrives via `opts.body` and is folded in here.
fn parse_request_args(
    method_name: &str,
    args: mlua::MultiValue,
) -> mlua::Result<(String, String, bool, Option<Table>)> {
    let has_body = method_name != "get" && method_name != "delete";

    let mut args_iter = args.into_iter();
    let url: String = match args_iter.next() {
        Some(Value::String(s)) => s.to_str()?.to_string(),
        _ => {
            return Err(mlua::Error::runtime(format!(
                "http.{method_name}: first argument must be a URL string"
            )));
        }
    };

    let (mut body_str, mut auto_json, opts) = if has_body {
        let (body, is_json) = match args_iter.next() {
            Some(Value::String(s)) => (s.to_str()?.to_string(), false),
            Some(Value::Table(t)) => {
                let json_val = lua_table_to_json(&t)?;
                let serialized = serde_json::to_string(&json_val).map_err(|e| {
                    mlua::Error::runtime(format!("http.{method_name}: JSON encode failed: {e}"))
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

    // RFC 7231 permits a body on DELETE; some assay-* admin endpoints
    // (e.g. `DELETE /admin/auth/zanzibar/tuples`) require a JSON body
    // to identify which row to remove. The Lua DELETE shorthand only
    // accepts `(url, opts)`, so we surface a body via `opts.body`
    // (string OR table for auto-JSON). `Content-Type: application/json`
    // is set automatically when a table is passed, mirroring `http.post`.
    if !has_body
        && let Some(ref opts_table) = opts
        && let Ok(body_val) = opts_table.get::<Value>("body")
    {
        match body_val {
            Value::String(s) => body_str = s.to_str()?.to_string(),
            Value::Table(t) => {
                let json_val = lua_table_to_json(&t)?;
                let serialized = serde_json::to_string(&json_val).map_err(|e| {
                    mlua::Error::runtime(format!("http.{method_name}: JSON encode failed: {e}"))
                })?;
                body_str = serialized;
                auto_json = true;
            }
            Value::Nil => {}
            _ => {
                return Err(mlua::Error::runtime(format!(
                    "http.{method_name}: opts.body must be a string, table, or nil"
                )));
            }
        }
    }

    Ok((url, body_str, auto_json, opts))
}

fn build_request(
    client: &reqwest::Client,
    method_name: &str,
    url: &str,
    body_str: String,
    auto_json: bool,
    opts: Option<&Table>,
) -> mlua::Result<reqwest::RequestBuilder> {
    let mut req = match method_name {
        "get" => client.get(url),
        "post" => client.post(url),
        "put" => client.put(url),
        "patch" => client.patch(url),
        "delete" => client.delete(url),
        _ => {
            return Err(mlua::Error::runtime(format!(
                "http: unsupported method: {method_name}"
            )));
        }
    };

    if !body_str.is_empty() {
        req = req.body(body_str);
    }
    if auto_json {
        req = req.header("Content-Type", "application/json");
    }
    // Caller headers replace the runtime's rather than adding a second value.
    // `RequestBuilder::header` appends, so a module naming `Content-Type` —
    // the obvious thing to do, and what `assay.openstack` did — sent it twice
    // and Keystone rejected the request. `headers` replaces per name.
    if let Some(opts_table) = opts
        && let Ok(headers_table) = opts_table.get::<Table>("headers")
    {
        let mut caller_headers = reqwest::header::HeaderMap::new();
        for pair in headers_table.pairs::<String, String>() {
            let (k, v) = pair?;
            let name = reqwest::header::HeaderName::try_from(k.as_str()).map_err(|e| {
                mlua::Error::runtime(format!(
                    "http.{method_name}: invalid header name {k:?}: {e}"
                ))
            })?;
            let value = reqwest::header::HeaderValue::try_from(v.as_str()).map_err(|e| {
                mlua::Error::runtime(format!(
                    "http.{method_name}: invalid value for header {k:?}: {e}"
                ))
            })?;
            caller_headers.insert(name, value);
        }
        req = req.headers(caller_headers);
    }

    Ok(req)
}

fn headers_to_lua(lua: &Lua, headers: &reqwest::header::HeaderMap) -> mlua::Result<Table> {
    let headers_out = lua.create_table()?;
    for (name, value) in headers {
        if let Ok(v) = value.to_str() {
            headers_out.set(name.as_str().to_string(), v.to_string())?;
        }
    }
    Ok(headers_out)
}

/// Drives an `text/event-stream` body, invoking `callback` per event until the
/// stream ends or the callback answers `"close"`.
async fn stream_sse_events(
    lua: &Lua,
    method_name: &str,
    resp: reqwest::Response,
    callback: mlua::Function,
    result: Table,
) -> mlua::Result<Value> {
    {
        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();

        use futures_util::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| {
                mlua::Error::runtime(format!("http.{method_name}: SSE stream error: {e}"))
            })?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Parse complete SSE events (delimited by double newline)
            while let Some(pos) = buffer.find("\n\n") {
                let event_text = buffer[..pos].to_string();
                buffer = buffer[pos + 2..].to_string();

                if event_text.trim().is_empty() {
                    continue;
                }

                let event_table = lua.create_table()?;
                for line in event_text.lines() {
                    if let Some(value) = line.strip_prefix("event: ") {
                        event_table.set("event", value.to_string())?;
                    } else if let Some(value) = line.strip_prefix("data: ") {
                        event_table.set("data", value.to_string())?;
                    } else if let Some(value) = line.strip_prefix("id: ") {
                        event_table.set("id", value.to_string())?;
                    } else if let Some(value) = line.strip_prefix("retry: ")
                        && let Ok(ms) = value.parse::<i64>()
                    {
                        event_table.set("retry", ms)?;
                    }
                }

                let action: Value = callback.call_async(Value::Table(event_table)).await?;
                // If callback returns "close", stop streaming
                if let Value::String(s) = &action
                    && s.to_str()? == "close"
                {
                    return Ok(Value::Table(result));
                }
            }
        }

        Ok(Value::Table(result))
    }
}

async fn execute_http_request(
    lua: &Lua,
    client: &reqwest::Client,
    method_name: &str,
    args: mlua::MultiValue,
) -> mlua::Result<Value> {
    let (url, body_str, auto_json, opts) = parse_request_args(method_name, args)?;
    let req = build_request(
        client,
        method_name,
        &url,
        body_str,
        auto_json,
        opts.as_ref(),
    )?;

    let resp = req
        .send()
        .await
        .map_err(|e| mlua::Error::runtime(format!("http.{method_name} failed: {e}")))?;
    let status = resp.status().as_u16();
    let resp_headers = resp.headers().clone();

    let result = lua.create_table()?;
    result.set("status", status)?;
    result.set("headers", headers_to_lua(lua, &resp_headers)?)?;

    let is_sse = resp_headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));
    let on_event_callback = opts
        .as_ref()
        .and_then(|o| o.get::<mlua::Function>("on_event").ok());

    if let (true, Some(callback)) = (is_sse, on_event_callback) {
        return stream_sse_events(lua, method_name, resp, callback, result).await;
    }

    // Buffer the full body as raw bytes (not `.text()`) so binary payloads —
    // gzip/xz/zstd, images, tarballs — round-trip cleanly. Lua strings in mlua
    // are byte buffers, so text-decoding callers are unaffected.
    let body_bytes = resp.bytes().await.map_err(|e| {
        mlua::Error::runtime(format!("http.{method_name}: reading body failed: {e}"))
    })?;
    result.set("body", lua.create_string(&body_bytes)?)?;

    Ok(Value::Table(result))
}
