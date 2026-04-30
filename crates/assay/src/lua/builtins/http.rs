use super::json::lua_table_to_json;
#[cfg(feature = "server")]
use super::json::lua_value_to_json;
#[cfg(feature = "server")]
use http_body_util::Full;
#[cfg(feature = "server")]
use hyper::body::{Bytes, Frame, Incoming};
#[cfg(feature = "server")]
use hyper::server::conn::http1;
#[cfg(feature = "server")]
use hyper::service::service_fn;
#[cfg(feature = "server")]
use hyper::{Request, Response, StatusCode};
use mlua::{Lua, Table, UserData, Value};
#[cfg(feature = "server")]
use std::cell::RefCell;
#[cfg(feature = "server")]
use std::collections::HashMap;
#[cfg(feature = "server")]
use std::pin::Pin;
#[cfg(feature = "server")]
use std::rc::Rc;
#[cfg(feature = "server")]
use std::task::{Context, Poll};
#[cfg(feature = "server")]
use tokio::net::TcpListener;
#[cfg(feature = "server")]
use tracing::error;

/// Public newtype wrapping an [`axum::Router`] so it can round-trip through
/// the Lua VM as a [`mlua::AnyUserData`].
///
/// Downstream binaries build a Rust-side `axum::Router` (typically
/// holding `assay-engine` HTTP routes), wrap it in this type, stash it in
/// a Lua global (or pass it positionally), and the Lua-defined
/// `http.serve_with_extra(port, routes, extra)` builtin pulls the router
/// back out and folds its routes into the dispatcher.
///
/// The type is intentionally a tuple-struct with a `pub` inner so callers
/// can construct one trivially: `LuaAxumRouter(my_router)`.
#[cfg(feature = "server")]
#[derive(Clone)]
pub struct LuaAxumRouter(pub axum::Router);

#[cfg(feature = "server")]
impl mlua::UserData for LuaAxumRouter {}

struct HttpClient(reqwest::Client);
impl UserData for HttpClient {}

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

    #[cfg(feature = "server")]
    let serve_fn = lua.create_async_function(|lua, args: mlua::MultiValue| async move {
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

        // Expose the actual bound port so callers using port 0 can discover it
        let actual_port = listener
            .local_addr()
            .map_err(|e| {
                mlua::Error::runtime(format!("http.serve: failed to get local addr: {e}"))
            })?
            .port();
        lua.globals().set("_SERVER_PORT", actual_port)?;

        loop {
            let (stream, addr) = listener
                .accept()
                .await
                .map_err(|e| mlua::Error::runtime(format!("http.serve: accept failed: {e}")))?;
            let peer_addr = addr.to_string();

            let routes = routes.clone();
            let lua_clone = lua.clone();

            tokio::task::spawn_local(async move {
                let io = hyper_util::rt::TokioIo::new(stream);
                let routes = routes.clone();
                let lua = lua_clone.clone();
                let peer_addr = peer_addr.clone();

                let service = service_fn(move |req: Request<Incoming>| {
                    let routes = routes.clone();
                    let lua = lua.clone();
                    let peer_addr = peer_addr.clone();
                    async move { handle_request(&lua, &routes, None, peer_addr, req).await }
                });

                if let Err(e) = http1::Builder::new()
                    .serve_connection(io, service)
                    .with_upgrades()
                    .await
                    && !e.to_string().contains("connection closed")
                {
                    error!("http.serve: connection error: {e}");
                }
            });
        }
    })?;
    #[cfg(feature = "server")]
    http_table.set("serve", serve_fn)?;

    // ── http.serve_with_extra(port, routes_table, extra_router) ───────────────
    //
    // Same shape as `http.serve` plus a third argument: a [`LuaAxumRouter`]
    // userdata wrapping a Rust-built `axum::Router`. Lua-defined routes are
    // matched first; on miss, the request is forwarded to the extra
    // `axum::Router` (which can produce 404 itself if it doesn't match either).
    //
    // Precedence: Lua wins. If the same path is defined by both, the Lua
    // handler is invoked. This is the inverse of `axum::Router::merge` (where
    // a duplicate panics) and was chosen because the Lua side is the
    // existing surface — the extra router is purely additive routes the
    // host binary contributes (typically engine APIs under a non-overlapping
    // path prefix like `/api/v1/engine/*`).
    //
    // The extra router is cloned per-connection (`axum::Router: Clone` is a
    // shallow `Arc` clone — cheap).
    #[cfg(feature = "server")]
    let serve_with_extra_fn = lua.create_async_function(|lua, args: mlua::MultiValue| async move {
        let mut args_iter = args.into_iter();

        let port: u16 = match args_iter.next() {
            Some(Value::Integer(n)) => n as u16,
            _ => {
                return Err::<(), _>(mlua::Error::runtime(
                    "http.serve_with_extra: first argument must be a port number",
                ));
            }
        };

        let routes_table = match args_iter.next() {
            Some(Value::Table(t)) => t,
            _ => {
                return Err::<(), _>(mlua::Error::runtime(
                    "http.serve_with_extra: second argument must be a routes table",
                ));
            }
        };

        let extra_router: axum::Router = match args_iter.next() {
            Some(Value::UserData(ud)) => {
                let r = ud.borrow::<LuaAxumRouter>().map_err(|_| {
                    mlua::Error::runtime(
                        "http.serve_with_extra: third argument must be a LuaAxumRouter userdata",
                    )
                })?;
                r.0.clone()
            }
            _ => {
                return Err::<(), _>(mlua::Error::runtime(
                    "http.serve_with_extra: third argument must be a LuaAxumRouter userdata",
                ));
            }
        };

        let routes = Rc::new(parse_routes(&routes_table)?);

        let listener = TcpListener::bind(format!("0.0.0.0:{port}"))
            .await
            .map_err(|e| {
                mlua::Error::runtime(format!("http.serve_with_extra: bind failed: {e}"))
            })?;

        let actual_port = listener
            .local_addr()
            .map_err(|e| {
                mlua::Error::runtime(format!(
                    "http.serve_with_extra: failed to get local addr: {e}"
                ))
            })?
            .port();
        lua.globals().set("_SERVER_PORT", actual_port)?;

        loop {
            let (stream, addr) = listener.accept().await.map_err(|e| {
                mlua::Error::runtime(format!("http.serve_with_extra: accept failed: {e}"))
            })?;
            let peer_addr = addr.to_string();

            let routes = routes.clone();
            let lua_clone = lua.clone();
            let extra_router = extra_router.clone();

            tokio::task::spawn_local(async move {
                let io = hyper_util::rt::TokioIo::new(stream);
                let routes = routes.clone();
                let lua = lua_clone.clone();
                let peer_addr = peer_addr.clone();
                let extra_router = extra_router.clone();

                let service = service_fn(move |req: Request<Incoming>| {
                    let routes = routes.clone();
                    let lua = lua.clone();
                    let peer_addr = peer_addr.clone();
                    let extra_router = extra_router.clone();
                    async move {
                        handle_request(&lua, &routes, Some(extra_router), peer_addr, req).await
                    }
                });

                if let Err(e) = http1::Builder::new()
                    .serve_connection(io, service)
                    .with_upgrades()
                    .await
                    && !e.to_string().contains("connection closed")
                {
                    error!("http.serve_with_extra: connection error: {e}");
                }
            });
        }
    })?;
    #[cfg(feature = "server")]
    http_table.set("serve_with_extra", serve_with_extra_fn)?;

    // http.download(url, path, opts?) -> bytes_written
    // Streams the response body to disk via a temp file, then atomic-renames into place.
    // Creates parent directories as needed. On any failure (4xx/5xx, IO error, network),
    // the temp file is removed and the error propagates — no partial file at `path`.
    let download_client = client.clone();
    let download_fn = lua.create_async_function(move |_, args: mlua::MultiValue| {
        let client = download_client.clone();
        async move {
            use futures_util::StreamExt;
            use tokio::io::AsyncWriteExt;

            let mut args_iter = args.into_iter();
            let url: String = match args_iter.next() {
                Some(mlua::Value::String(s)) => s.to_str()?.to_string(),
                _ => return Err(mlua::Error::runtime("http.download: first arg must be url string")),
            };
            let path: String = match args_iter.next() {
                Some(mlua::Value::String(s)) => s.to_str()?.to_string(),
                _ => return Err(mlua::Error::runtime("http.download: second arg must be dest path string")),
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

            // Ensure parent dir
            if let Some(parent) = std::path::Path::new(&path).parent()
                && !parent.as_os_str().is_empty()
            {
                tokio::fs::create_dir_all(parent).await.map_err(|e| {
                    mlua::Error::runtime(format!("http.download: mkdir parent: {e}"))
                })?;
            }

            // Open temp file at <path>.tmp.<pid>
            let tmp = format!("{path}.tmp.{}", std::process::id());
            let mut file = tokio::fs::File::create(&tmp).await.map_err(|e| {
                mlua::Error::runtime(format!("http.download: create temp {tmp:?}: {e}"))
            })?;

            // Cleanup helper closure result
            let do_download = async {
                let resp = req.send().await.map_err(|e| {
                    mlua::Error::runtime(format!("http.download: request: {e}"))
                })?;
                if !resp.status().is_success() {
                    return Err(mlua::Error::runtime(format!(
                        "http.download: HTTP {} for {url}",
                        resp.status()
                    )));
                }
                let mut total: i64 = 0;
                let mut stream = resp.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    let bytes = chunk.map_err(|e| {
                        mlua::Error::runtime(format!("http.download: stream: {e}"))
                    })?;
                    total += bytes.len() as i64;
                    file.write_all(&bytes).await.map_err(|e| {
                        mlua::Error::runtime(format!("http.download: write: {e}"))
                    })?;
                }
                file.flush().await.map_err(|e| {
                    mlua::Error::runtime(format!("http.download: flush: {e}"))
                })?;
                file.sync_all().await.map_err(|e| {
                    mlua::Error::runtime(format!("http.download: fsync: {e}"))
                })?;
                drop(file); // close before rename on Windows; harmless on Linux
                Ok(total)
            };

            match do_download.await {
                Ok(total) => {
                    tokio::fs::rename(&tmp, &path).await.map_err(|e| {
                        mlua::Error::runtime(format!("http.download: rename {tmp:?} -> {path:?}: {e}"))
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

    lua.globals().set("http", http_table)?;
    Ok(())
}

async fn execute_http_request(
    lua: &Lua,
    client: &reqwest::Client,
    method_name: &str,
    args: mlua::MultiValue,
) -> mlua::Result<Value> {
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

    let mut req = match method_name {
        "get" => client.get(&url),
        "post" => client.post(&url),
        "put" => client.put(&url),
        "patch" => client.patch(&url),
        "delete" => client.delete(&url),
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
    if let Some(ref opts_table) = opts
        && let Ok(headers_table) = opts_table.get::<Table>("headers")
    {
        for pair in headers_table.pairs::<String, String>() {
            let (k, v) = pair?;
            req = req.header(k, v);
        }
    }

    let resp = req
        .send()
        .await
        .map_err(|e| mlua::Error::runtime(format!("http.{method_name} failed: {e}")))?;
    let status = resp.status().as_u16();
    let resp_headers = resp.headers().clone();

    // Check for SSE: Content-Type text/event-stream + on_event callback
    let is_sse = resp_headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));

    let on_event_callback = opts
        .as_ref()
        .and_then(|o| o.get::<mlua::Function>("on_event").ok());

    if let (true, Some(callback)) = (is_sse, on_event_callback) {

        let result = lua.create_table()?;
        result.set("status", status)?;
        let headers_out = lua.create_table()?;
        for (name, value) in &resp_headers {
            if let Ok(v) = value.to_str() {
                headers_out.set(name.as_str().to_string(), v.to_string())?;
            }
        }
        result.set("headers", headers_out)?;

        // Stream SSE events to the callback
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

        return Ok(Value::Table(result));
    }

    // Standard response: buffer full body. Use raw bytes (not .text()) so binary
    // payloads — gzip/xz/zstd, images, tarballs — round-trip cleanly. Lua strings
    // in mlua are byte buffers, so this remains compatible with existing
    // text-decoding callers.
    let body_bytes = resp.bytes().await.map_err(|e| {
        mlua::Error::runtime(format!("http.{method_name}: reading body failed: {e}"))
    })?;
    let body = lua.create_string(&body_bytes)?;

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

/// A streaming body backed by an mpsc channel, used for SSE responses.
#[cfg(feature = "server")]
struct SseBody {
    rx: tokio::sync::mpsc::Receiver<Bytes>,
}

#[cfg(feature = "server")]
impl hyper::body::Body for SseBody {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(bytes)) => Poll::Ready(Some(Ok(Frame::data(bytes)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Format a Lua table with optional `event`, `data`, `id`, `retry` fields into an SSE text block.
#[cfg(feature = "server")]
fn format_sse_event(event_table: &Table) -> mlua::Result<String> {
    let mut out = String::new();

    if let Ok(Some(event)) = event_table.get::<Option<String>>("event") {
        if event.contains('\n') || event.contains('\r') {
            return Err(mlua::Error::runtime(
                "SSE event name must not contain newlines",
            ));
        }
        out.push_str("event: ");
        out.push_str(&event);
        out.push('\n');
    }
    if let Ok(Some(data)) = event_table.get::<Option<String>>("data") {
        // SSE spec: each line of data gets its own "data:" prefix
        for line in data.split('\n') {
            out.push_str("data: ");
            out.push_str(line);
            out.push('\n');
        }
    }
    if let Ok(Some(id)) = event_table.get::<Option<String>>("id") {
        if id.contains('\n') || id.contains('\r') {
            return Err(mlua::Error::runtime("SSE id must not contain newlines"));
        }
        out.push_str("id: ");
        out.push_str(&id);
        out.push('\n');
    }
    if let Ok(Some(retry)) = event_table.get::<Option<i64>>("retry") {
        out.push_str("retry: ");
        out.push_str(&retry.to_string());
        out.push('\n');
    }

    // SSE events are terminated by a blank line
    out.push('\n');
    Ok(out)
}

#[cfg(feature = "server")]
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

/// Unified response body type.
///
/// Originally `Either<Full<Bytes>, SseBody>`; widened to [`axum::body::Body`]
/// so we can transparently forward responses produced by an external
/// `axum::Router` (see `http.serve_with_extra`). `axum::body::Body` is a
/// thin wrapper over `UnsyncBoxBody<Bytes, axum::Error>` and accepts any
/// `http_body::Body<Data = Bytes>` via [`axum::body::Body::new`], so all
/// existing body shapes (static `Full<Bytes>`, the SSE channel body) still
/// fit; only the construction surface changes.
#[cfg(feature = "server")]
type ServerBody = axum::body::Body;

#[cfg(feature = "server")]
fn lookup_route<'a>(
    routes: &'a HashMap<(String, String), mlua::Function>,
    method: &str,
    path: &str,
) -> Option<&'a mlua::Function> {
    let key = (method.to_string(), path.to_string());
    if let Some(f) = routes.get(&key) {
        return Some(f);
    }
    let mut search = path;
    while let Some(pos) = search.rfind('/') {
        let prefix = &search[..pos];
        let wildcard_key = (method.to_string(), format!("{prefix}/*"));
        if let Some(f) = routes.get(&wildcard_key) {
            return Some(f);
        }
        if pos == 0 {
            let root_key = (method.to_string(), "/*".to_string());
            return routes.get(&root_key);
        }
        search = prefix;
    }
    None
}

#[cfg(feature = "server")]
fn is_websocket_upgrade(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("upgrade") && v.to_ascii_lowercase().contains("websocket"))
}

#[cfg(feature = "server")]
fn validate_ws_request(headers: &[(String, String)]) -> Result<String, &'static str> {
    let mut has_connection_upgrade = false;
    let mut version_ok = false;
    let mut key: Option<String> = None;
    for (k, v) in headers {
        match k.to_ascii_lowercase().as_str() {
            "connection" if v.to_ascii_lowercase().contains("upgrade") => {
                has_connection_upgrade = true;
            }
            "sec-websocket-version" if v.trim() == "13" => {
                version_ok = true;
            }
            "sec-websocket-key" => {
                key = Some(v.clone());
            }
            _ => {}
        }
    }
    if !has_connection_upgrade {
        return Err("missing Connection: Upgrade header");
    }
    if !version_ok {
        return Err("Sec-WebSocket-Version must be 13");
    }
    key.ok_or("missing Sec-WebSocket-Key header")
}

#[cfg(feature = "server")]
fn compute_ws_accept(key: &str) -> String {
    use sha1::Digest;
    const MAGIC: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let mut hasher = sha1::Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(MAGIC);
    let digest = hasher.finalize();
    data_encoding::BASE64.encode(&digest)
}

/// Forward a `hyper` request into an `axum::Router` (used as a [`tower::Service`])
/// and return its response.
///
/// `axum::Router` implements `Service<Request<B>>` for any `B` that is an
/// `HttpBody<Data = Bytes>`, which `hyper::body::Incoming` is. The router's
/// response body is `axum::body::Body`, which is exactly our unified
/// [`ServerBody`].
///
/// The router's `Service::call` signature is `Infallible`, so the only way
/// this can fail is if there's a `hyper::Error` upstream — there isn't —
/// hence the `unreachable!()` on the error branch.
#[cfg(feature = "server")]
async fn forward_to_axum_router(
    mut router: axum::Router,
    req: Request<Incoming>,
) -> Result<Response<ServerBody>, hyper::Error> {
    use tower::Service;
    // `axum::Router::poll_ready` is `Poll::Ready(Ok(()))`, so we can call
    // straight through without driving readiness. We rely on the
    // `Service<Request<B>>` impl (not the `Service<IncomingStream>` one)
    // which is selected by the `Request<Incoming>` argument type.
    match <axum::Router as Service<Request<Incoming>>>::call(&mut router, req).await {
        Ok(resp) => Ok(resp),
        Err(_) => unreachable!("axum::Router::call is Infallible"),
    }
}

#[cfg(feature = "server")]
async fn handle_request(
    lua: &Lua,
    routes: &HashMap<(String, String), mlua::Function>,
    extra_router: Option<axum::Router>,
    peer_addr: String,
    req: Request<Incoming>,
) -> Result<Response<ServerBody>, hyper::Error> {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
        .collect();

    let is_ws = is_websocket_upgrade(&headers);

    let handler = match lookup_route(routes, &method, &path) {
        Some(h) => h.clone(),
        None => {
            // Lua dispatch missed. If an extra `axum::Router` was supplied via
            // `http.serve_with_extra`, hand the request off to it so its routes
            // (typically Rust-built, e.g. `assay-engine`'s `/api/v1/engine/*`)
            // can produce the response. Otherwise, fall back to a 404.
            if let Some(router) = extra_router {
                return forward_to_axum_router(router, req).await;
            }
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("content-type", "text/plain")
                .body(axum::body::Body::new(Full::new(Bytes::from("not found"))))
                .unwrap());
        }
    };

    if is_ws {
        let lua_resp = match build_lua_request_and_call(
            lua, &handler, &method, &path, &query, &headers, "",
        )
        .await
        {
            Ok(t) => t,
            Err(e) => {
                return Ok(Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .header("content-type", "text/plain")
                    .body(axum::body::Body::new(Full::new(Bytes::from(format!(
                        "handler error: {e}"
                    )))))
                    .unwrap());
            }
        };

        if let Ok(Some(ws_fn)) = lua_resp.get::<Option<mlua::Function>>("ws") {
            return build_ws_upgrade_response(lua, &headers, lua_resp, ws_fn, peer_addr, req);
        }

        return lua_response_to_http(lua, &lua_resp);
    }

    let body_bytes = match http_body_util::BodyExt::collect(req.into_body()).await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => Bytes::new(),
    };
    let body_str = String::from_utf8_lossy(&body_bytes).to_string();

    match build_lua_request_and_call(lua, &handler, &method, &path, &query, &headers, &body_str)
        .await
    {
        Ok(lua_resp) => lua_response_to_http(lua, &lua_resp),
        Err(e) => Ok(Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header("content-type", "text/plain")
            .body(axum::body::Body::new(Full::new(Bytes::from(format!(
                "handler error: {e}"
            )))))
            .unwrap()),
    }
}

#[cfg(feature = "server")]
fn build_ws_upgrade_response(
    lua: &Lua,
    headers: &[(String, String)],
    resp_table: Table,
    ws_fn: mlua::Function,
    peer_addr: String,
    req: Request<Incoming>,
) -> Result<Response<ServerBody>, hyper::Error> {
    let key = match validate_ws_request(headers) {
        Ok(k) => k,
        Err(msg) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("content-type", "text/plain")
                .body(axum::body::Body::new(Full::new(Bytes::from(format!(
                    "websocket upgrade rejected: {msg}"
                )))))
                .unwrap());
        }
    };
    let accept = compute_ws_accept(&key);

    let mut builder = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(hyper::header::UPGRADE, "websocket")
        .header(hyper::header::CONNECTION, "Upgrade")
        .header("sec-websocket-accept", accept);

    // User-supplied headers (e.g., for auth or tracing). Skip the protocol-controlled ones
    // we just set, in case the handler accidentally returns them too.
    if let Ok(Some(headers_table)) = resp_table.get::<Option<Table>>("headers") {
        for pair in headers_table.pairs::<String, mlua::String>().flatten() {
            let (k, v) = pair;
            let kl = k.to_ascii_lowercase();
            if matches!(
                kl.as_str(),
                "upgrade" | "connection" | "sec-websocket-accept"
            ) {
                continue;
            }
            if let Ok(s) = v.to_str() {
                builder = builder.header(&k, s.as_ref());
            }
        }
    }

    let response = builder
        .body(axum::body::Body::new(Full::new(Bytes::new())))
        .unwrap();

    let lua_clone = lua.clone();
    tokio::task::spawn_local(async move {
        let upgraded = match hyper::upgrade::on(req).await {
            Ok(u) => u,
            Err(e) => {
                error!("http.serve: ws upgrade failed: {e}");
                return;
            }
        };
        let io = hyper_util::rt::TokioIo::new(upgraded);
        let stream = tokio_tungstenite::WebSocketStream::from_raw_socket(
            io,
            tokio_tungstenite::tungstenite::protocol::Role::Server,
            None,
        )
        .await;
        let conn = super::ws::WsServerConn::new(stream, peer_addr);
        let ud = match lua_clone.create_userdata(conn) {
            Ok(u) => u,
            Err(e) => {
                error!("http.serve: ws userdata creation failed: {e}");
                return;
            }
        };
        if let Err(e) = ws_fn.call_async::<()>(ud).await
            && !e.to_string().contains("conn:read: ")
        {
            error!("http.serve: ws handler error: {e}");
        }
    });

    Ok(response)
}

#[cfg(feature = "server")]
async fn build_lua_request_and_call(
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

    // Parse query string into a params table with URL-decoded keys and values
    // (e.g. "a=1&b=hello%20world" -> {a="1", b="hello world"}).
    // Uses form_urlencoded which handles percent-encoding and `+` -> ` ` correctly,
    // so consumers like assay.ory.hydra get the raw value back rather than a doubly-encoded string.
    let params_table = lua.create_table()?;
    if !query.is_empty() {
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            params_table.set(key.into_owned(), value.into_owned())?;
        }
    }
    req_table.set("params", params_table)?;

    let headers_table = lua.create_table()?;
    for (k, v) in headers {
        headers_table.set(k.as_str(), v.as_str())?;
    }
    req_table.set("headers", headers_table)?;

    handler.call_async::<Table>(req_table).await
}

#[cfg(feature = "server")]
fn lua_response_to_http(
    lua: &Lua,
    resp_table: &Table,
) -> Result<Response<ServerBody>, hyper::Error> {
    let status = resp_table
        .get::<Option<u16>>("status")
        .unwrap_or(None)
        .unwrap_or(200);

    // Check for SSE function first
    if let Ok(Some(sse_fn)) = resp_table.get::<Option<mlua::Function>>("sse") {
        let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(32);

        let mut builder =
            Response::builder().status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK));

        // Apply custom headers first so they take precedence over SSE defaults
        if let Ok(Some(headers_table)) = resp_table.get::<Option<Table>>("headers") {
            for pair in headers_table.pairs::<String, Value>().flatten() {
                let (k, v) = pair;
                match v {
                    Value::String(s) => {
                        if let Ok(s) = s.to_str() {
                            builder = builder.header(&k, s.as_ref());
                        }
                    }
                    Value::Table(t) => {
                        // Array of strings → multiple headers with the same name
                        // (required for Set-Cookie when setting multiple cookies)
                        for val in t.sequence_values::<String>().flatten() {
                            builder = builder.header(&k, val);
                        }
                    }
                    _ => {}
                }
            }
        }

        let mut response = builder.body(axum::body::Body::new(SseBody { rx })).unwrap();
        let response_headers = response.headers_mut();
        if !response_headers.contains_key(hyper::header::CONTENT_TYPE) {
            response_headers.insert(
                hyper::header::CONTENT_TYPE,
                hyper::header::HeaderValue::from_static("text/event-stream"),
            );
        }
        if !response_headers.contains_key(hyper::header::CACHE_CONTROL) {
            response_headers.insert(
                hyper::header::CACHE_CONTROL,
                hyper::header::HeaderValue::from_static("no-cache"),
            );
        }
        if !response_headers.contains_key(hyper::header::CONNECTION) {
            response_headers.insert(
                hyper::header::CONNECTION,
                hyper::header::HeaderValue::from_static("keep-alive"),
            );
        }

        // Spawn the SSE function on the local set.
        // We wrap tx in Rc<RefCell<Option>> so we can explicitly close the channel
        // after the SSE function returns. If tx were only inside the Lua closure,
        // it would stay alive as long as Lua's GC keeps the closure, preventing the
        // channel from closing and the response body from completing.
        let lua_clone = lua.clone();
        tokio::task::spawn_local(async move {
            let tx_holder: Rc<RefCell<Option<tokio::sync::mpsc::Sender<Bytes>>>> =
                Rc::new(RefCell::new(Some(tx)));
            let tx_for_fn = tx_holder.clone();

            let send_fn = match lua_clone.create_async_function(move |_lua, event_table: Table| {
                let tx_ref = tx_for_fn.clone();
                async move {
                    let formatted = format_sse_event(&event_table)?;
                    let tx = tx_ref
                        .borrow()
                        .clone()
                        .ok_or_else(|| mlua::Error::runtime("SSE stream closed"))?;
                    if tx.send(Bytes::from(formatted)).await.is_err() {
                        return Err(mlua::Error::runtime("SSE stream closed"));
                    }
                    Ok(())
                }
            }) {
                Ok(f) => f,
                Err(e) => {
                    error!("http.serve SSE: failed to create send callback: {e}");
                    return;
                }
            };

            if let Err(e) = sse_fn.call_async::<()>(send_fn).await
                && !e.to_string().contains("SSE stream closed")
            {
                error!("http.serve SSE: handler error: {e}");
            }

            // Explicitly close the channel so the response body completes
            tx_holder.borrow_mut().take();
        });

        return Ok(response);
    }

    let mut builder =
        Response::builder().status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK));

    let has_content_type =
        if let Ok(Some(headers_table)) = resp_table.get::<Option<Table>>("headers") {
            let mut found_ct = false;
            for pair in headers_table.pairs::<String, Value>().flatten() {
                let (k, v) = pair;
                if k.eq_ignore_ascii_case("content-type") {
                    found_ct = true;
                }
                match v {
                    Value::String(s) => {
                        if let Ok(s) = s.to_str() {
                            builder = builder.header(&k, s.as_ref());
                        }
                    }
                    Value::Table(t) => {
                        // Array of strings → multiple headers with the same name
                        // (required for Set-Cookie when setting multiple cookies)
                        for val in t.sequence_values::<String>().flatten() {
                            builder = builder.header(&k, val);
                        }
                    }
                    _ => {}
                }
            }
            found_ct
        } else {
            false
        };

    let body_bytes = if let Ok(Some(json_table)) = resp_table.get::<Option<Table>>("json") {
        let json_val =
            lua_value_to_json(&Value::Table(json_table)).unwrap_or(serde_json::Value::Null);
        let serialized = serde_json::to_string(&json_val).unwrap_or_else(|_| "null".to_string());
        if !has_content_type {
            builder = builder.header("content-type", "application/json");
        }
        Bytes::from(serialized)
    } else if let Ok(Some(body_lua)) = resp_table.get::<Option<mlua::String>>("body") {
        if !has_content_type {
            builder = builder.header("content-type", "text/plain");
        }
        Bytes::from(body_lua.as_bytes().to_vec())
    } else {
        if !has_content_type {
            builder = builder.header("content-type", "text/plain");
        }
        Bytes::new()
    };

    Ok(builder.body(axum::body::Body::new(Full::new(body_bytes))).unwrap())
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;
    use axum::Router;
    use axum::routing::get;
    use mlua::Lua;

    /// `LuaAxumRouter` wraps an `axum::Router` so it can be passed through Lua
    /// as `mlua` userdata. This test mirrors a typical downstream embedding
    /// pattern: build a Router in Rust, wrap it in `LuaAxumRouter`, hand it
    /// to the Lua VM via `create_userdata`, stash it as a global, then read
    /// it back out and confirm the round-trip preserves the underlying type.
    #[test]
    fn lua_axum_router_round_trips_through_mlua_globals() {
        let lua = Lua::new();
        let router = Router::new().route("/ping", get(|| async { "pong" }));
        let wrapped = LuaAxumRouter(router);

        let ud = lua
            .create_userdata(wrapped)
            .expect("create_userdata for LuaAxumRouter");
        lua.globals()
            .set("EXTRA_ROUTER", ud)
            .expect("stash userdata in globals");

        let value: mlua::Value = lua
            .globals()
            .get("EXTRA_ROUTER")
            .expect("read userdata back from globals");
        let ud = match value {
            mlua::Value::UserData(u) => u,
            other => panic!("expected UserData, got {other:?}"),
        };
        let _borrowed = ud
            .borrow::<LuaAxumRouter>()
            .expect("downcast to LuaAxumRouter");
    }

    /// `Clone` on `LuaAxumRouter` should be cheap and preserve route
    /// dispatch — `axum::Router::clone` is a shallow `Arc` clone, so cloning
    /// the wrapper just clones that handle. We verify both clones still
    /// produce the same route at the type level (compile check).
    #[test]
    fn lua_axum_router_is_clone_and_preserves_routes() {
        let router = Router::<()>::new().route("/health", get(|| async { "ok" }));
        let wrapped = LuaAxumRouter(router);
        let _cloned = wrapped.clone();
        // If this compiles and `clone()` is callable, the public bound holds.
    }
}
