mod common;

use common::run_lua_local;

#[tokio::test]
async fn test_http_serve_get_body() {
    run_lua_local(
        r#"
        local server = async.spawn(function()
            http.serve(0, {
                GET = {
                    ["/health"] = function(req) return { status = 200, body = "ok" } end,
                }
            })
        end)
        sleep(0.1)
        local port = _SERVER_PORT
        local resp = http.get("http://127.0.0.1:" .. port .. "/health")
        assert.eq(resp.status, 200)
        assert.eq(resp.body, "ok")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_http_serve_post_body() {
    run_lua_local(
        r#"
        local server = async.spawn(function()
            http.serve(0, {
                POST = {
                    ["/submit"] = function(req)
                        return { status = 201, body = req.body }
                    end,
                }
            })
        end)
        sleep(0.1)
        local port = _SERVER_PORT
        local resp = http.post("http://127.0.0.1:" .. port .. "/submit", "hello world")
        assert.eq(resp.status, 201)
        assert.eq(resp.body, "hello world")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_http_serve_json_response() {
    run_lua_local(
        r#"
        local server = async.spawn(function()
            http.serve(0, {
                GET = {
                    ["/data"] = function(req)
                        return { status = 200, json = { items = {1, 2, 3} } }
                    end,
                }
            })
        end)
        sleep(0.1)
        local port = _SERVER_PORT
        local resp = http.get("http://127.0.0.1:" .. port .. "/data")
        assert.eq(resp.status, 200)
        assert.contains(resp.headers["content-type"], "application/json")
        local data = json.parse(resp.body)
        assert.eq(data.items[1], 1)
        assert.eq(data.items[2], 2)
        assert.eq(data.items[3], 3)
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_http_serve_custom_headers() {
    run_lua_local(
        r#"
        local server = async.spawn(function()
            http.serve(0, {
                GET = {
                    ["/custom"] = function(req)
                        return {
                            status = 200,
                            body = "with headers",
                            headers = { ["x-custom"] = "test-value" }
                        }
                    end,
                }
            })
        end)
        sleep(0.1)
        local port = _SERVER_PORT
        local resp = http.get("http://127.0.0.1:" .. port .. "/custom")
        assert.eq(resp.status, 200)
        assert.eq(resp.headers["x-custom"], "test-value")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_http_serve_404_unregistered() {
    run_lua_local(
        r#"
        local server = async.spawn(function()
            http.serve(0, {
                GET = {
                    ["/exists"] = function(req) return { body = "here" } end,
                }
            })
        end)
        sleep(0.1)
        local port = _SERVER_PORT
        local resp = http.get("http://127.0.0.1:" .. port .. "/missing")
        assert.eq(resp.status, 404)
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_http_serve_multiple_methods_same_path() {
    run_lua_local(
        r#"
        local server = async.spawn(function()
            http.serve(0, {
                GET = {
                    ["/resource"] = function(req) return { body = "get-result" } end,
                },
                POST = {
                    ["/resource"] = function(req) return { status = 201, body = "post-result" } end,
                }
            })
        end)
        sleep(0.1)
        local port = _SERVER_PORT
        local get_resp = http.get("http://127.0.0.1:" .. port .. "/resource")
        assert.eq(get_resp.status, 200)
        assert.eq(get_resp.body, "get-result")
        local post_resp = http.post("http://127.0.0.1:" .. port .. "/resource", "")
        assert.eq(post_resp.status, 201)
        assert.eq(post_resp.body, "post-result")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_http_serve_request_query() {
    run_lua_local(
        r#"
        local server = async.spawn(function()
            http.serve(0, {
                GET = {
                    ["/search"] = function(req)
                        return { body = req.query }
                    end,
                }
            })
        end)
        sleep(0.1)
        local port = _SERVER_PORT
        local resp = http.get("http://127.0.0.1:" .. port .. "/search?q=hello&page=1")
        assert.eq(resp.status, 200)
        assert.eq(resp.body, "q=hello&page=1")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_http_serve_request_headers() {
    run_lua_local(
        r#"
        local server = async.spawn(function()
            http.serve(0, {
                GET = {
                    ["/echo-header"] = function(req)
                        return { body = req.headers["x-test-header"] or "missing" }
                    end,
                }
            })
        end)
        sleep(0.1)
        local port = _SERVER_PORT
        local resp = http.get("http://127.0.0.1:" .. port .. "/echo-header", {
            headers = { ["x-test-header"] = "my-value" }
        })
        assert.eq(resp.status, 200)
        assert.eq(resp.body, "my-value")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_http_serve_sse() {
    run_lua_local(
        r#"
        local server = async.spawn(function()
            http.serve(0, {
                GET = {
                    ["/events"] = function(req)
                        return {
                            status = 200,
                            sse = function(send)
                                send({ data = "hello" })
                                send({ event = "update", data = "world" })
                                send({ event = "done", data = "bye", id = "3" })
                            end
                        }
                    end,
                }
            })
        end)
        sleep(0.2)
        local port = _SERVER_PORT
        local resp = http.get("http://127.0.0.1:" .. port .. "/events")
        assert.eq(resp.status, 200)
        assert.contains(resp.headers["content-type"], "text/event-stream")
        -- Verify SSE events are present in order
        local hello_idx = string.find(resp.body, "data: hello", 1, true)
        assert.ne(hello_idx, nil)
        local update_idx = string.find(resp.body, "event: update", hello_idx + 1, true)
        assert.ne(update_idx, nil)
        local world_idx = string.find(resp.body, "data: world", update_idx + 1, true)
        assert.ne(world_idx, nil)
        local done_idx = string.find(resp.body, "event: done", world_idx + 1, true)
        assert.ne(done_idx, nil)
        local bye_idx = string.find(resp.body, "data: bye", done_idx + 1, true)
        assert.ne(bye_idx, nil)
        local id_idx = string.find(resp.body, "id: 3", bye_idx + 1, true)
        assert.ne(id_idx, nil)
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_http_serve_custom_content_type() {
    run_lua_local(
        r#"
        local server = async.spawn(function()
            http.serve(0, {
                GET = {
                    ["/html"] = function(req)
                        return {
                            status = 200,
                            body = "<h1>ok</h1>",
                            headers = { ["content-type"] = "text/html" }
                        }
                    end,
                }
            })
        end)
        sleep(0.2)
        local port = _SERVER_PORT
        local resp = http.get("http://127.0.0.1:" .. port .. "/html")
        assert.eq(resp.status, 200)
        assert.eq(resp.headers["content-type"], "text/html")
        assert.eq(resp.body, "<h1>ok</h1>")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_http_serve_async_handler() {
    run_lua_local(
        r#"
        -- Start a simple backend server
        local backend = async.spawn(function()
            http.serve(0, {
                GET = {
                    ["/data"] = function(req)
                        return { status = 200, body = "backend-response" }
                    end,
                }
            })
        end)
        sleep(0.2)
        local backend_port = _SERVER_PORT

        -- Start a proxy server whose handler calls http.get (async inside handler)
        local proxy = async.spawn(function()
            http.serve(0, {
                GET = {
                    ["/proxy"] = function(req)
                        local resp = http.get("http://127.0.0.1:" .. backend_port .. "/data")
                        return {
                            status = resp.status,
                            body = "proxied: " .. resp.body
                        }
                    end,
                }
            })
        end)
        sleep(0.2)
        local proxy_port = _SERVER_PORT

        local resp = http.get("http://127.0.0.1:" .. proxy_port .. "/proxy")
        assert.eq(resp.status, 200)
        assert.eq(resp.body, "proxied: backend-response")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_http_serve_query_params() {
    run_lua_local(
        r#"
        local server = async.spawn(function()
            http.serve(0, {
                GET = {
                    ["/search"] = function(req)
                        return {
                            status = 200,
                            json = {
                                raw_query = req.query,
                                name = req.params.name,
                                page = req.params.page,
                            }
                        }
                    end,
                }
            })
        end)
        sleep(0.1)
        local port = _SERVER_PORT
        local resp = http.get("http://127.0.0.1:" .. port .. "/search?name=hello&page=2")
        assert.eq(resp.status, 200)
        local data = json.parse(resp.body)
        assert.eq(data.raw_query, "name=hello&page=2")
        assert.eq(data.name, "hello")
        assert.eq(data.page, "2")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_http_serve_url_encoded_query_params() {
    // Regression test: req.params must contain URL-decoded values, not raw
    // percent-encoded strings. Otherwise consumers that re-encode (e.g.
    // assay.hydra) end up double-encoding values like "g=" -> "g%3D" -> "g%253D".
    run_lua_local(
        r#"
        local server = async.spawn(function()
            http.serve(0, {
                GET = {
                    ["/echo"] = function(req)
                        return {
                            status = 200,
                            json = {
                                challenge = req.params.challenge,
                                space    = req.params.space,
                                plus     = req.params.plus,
                                eq       = req.params.eq,
                                unicode  = req.params.unicode,
                            }
                        }
                    end,
                }
            })
        end)
        sleep(0.1)
        local port = _SERVER_PORT
        -- challenge ends with `=` (base64 padding) URL-encoded as %3D
        -- space encoded as %20 and as `+`
        -- raw `=` mid-value, and a unicode char
        local resp = http.get("http://127.0.0.1:" .. port
            .. "/echo?challenge=abc%3D&space=hello%20world&plus=hello+world&eq=a%3Db&unicode=caf%C3%A9")
        assert.eq(resp.status, 200)
        local data = json.parse(resp.body)
        assert.eq(data.challenge, "abc=")
        assert.eq(data.space, "hello world")
        assert.eq(data.plus, "hello world")
        assert.eq(data.eq, "a=b")
        assert.eq(data.unicode, "café")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_http_serve_multi_value_header() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    // Do the raw TCP request inside the same LocalSet that runs the server,
    // otherwise the spawned server task stops being polled and the connection hangs.
    let vm = common::create_vm();
    let local = tokio::task::LocalSet::new();
    let buf: String = local
        .run_until(async {
            // Start the server
            let script = r#"
                async.spawn(function()
                    http.serve(0, {
                        GET = {
                            ["/multi-cookie"] = function(req)
                                return {
                                    status = 200,
                                    body = "ok",
                                    headers = {
                                        ["Set-Cookie"] = {
                                            "a=1; Path=/",
                                            "b=2; Path=/",
                                        },
                                    },
                                }
                            end,
                        }
                    })
                end)
                sleep(0.1)
                return _SERVER_PORT
            "#;
            let port: i64 = vm
                .load(assay::lua::async_bridge::strip_shebang(script))
                .eval_async()
                .await
                .unwrap();

            // Raw HTTP request to inspect multiple Set-Cookie headers
            let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
                .await
                .unwrap();
            stream
                .write_all(
                    b"GET /multi-cookie HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            let mut buf = String::new();
            tokio::time::timeout(std::time::Duration::from_secs(5), stream.read_to_string(&mut buf))
                .await
                .expect("timeout reading raw response")
                .unwrap();
            buf
        })
        .await;

    // Both Set-Cookie headers should be present in the raw response
    assert!(buf.contains("set-cookie: a=1"), "missing a=1 in: {buf}");
    assert!(buf.contains("set-cookie: b=2"), "missing b=2 in: {buf}");
}

#[tokio::test]
async fn test_http_serve_empty_query_params() {
    run_lua_local(
        r#"
        local server = async.spawn(function()
            http.serve(0, {
                GET = {
                    ["/noquery"] = function(req)
                        return {
                            status = 200,
                            json = {
                                raw_query = req.query,
                                has_params = next(req.params) ~= nil,
                            }
                        }
                    end,
                }
            })
        end)
        sleep(0.1)
        local port = _SERVER_PORT
        local resp = http.get("http://127.0.0.1:" .. port .. "/noquery")
        assert.eq(resp.status, 200)
        local data = json.parse(resp.body)
        assert.eq(data.raw_query, "")
        assert.eq(data.has_params, false)
    "#,
    )
    .await
    .unwrap();
}
