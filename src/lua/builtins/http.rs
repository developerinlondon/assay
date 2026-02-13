use super::json::{lua_table_to_json, lua_value_to_json};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use mlua::{Lua, Table, UserData, Value};
use std::collections::HashMap;
use std::rc::Rc;
use tokio::net::TcpListener;
use tracing::error;

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
            async move {
                execute_http_request(&lua, &client, &method_name, args).await
            }
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

        if let Some(ref opts_table) = opts {
            if let Ok(ca_path) = opts_table.get::<String>("ca_cert_file") {
                let pem = std::fs::read(&ca_path).map_err(|e| {
                    mlua::Error::runtime(format!(
                        "http.client: failed to read CA cert file {ca_path:?}: {e}"
                    ))
                })?;
                let cert = reqwest::Certificate::from_pem(&pem).map_err(|e| {
                    mlua::Error::runtime(format!(
                        "http.client: invalid PEM in {ca_path:?}: {e}"
                    ))
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

        loop {
            let (stream, _addr) = listener
                .accept()
                .await
                .map_err(|e| mlua::Error::runtime(format!("http.serve: accept failed: {e}")))?;

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

fn lua_response_to_http(resp_table: &Table) -> Result<Response<Full<Bytes>>, hyper::Error> {
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
