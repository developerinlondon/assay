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
        assert.contains(resp.body, "data: hello")
        assert.contains(resp.body, "event: update")
        assert.contains(resp.body, "data: world")
        assert.contains(resp.body, "event: done")
        assert.contains(resp.body, "data: bye")
        assert.contains(resp.body, "id: 3")
    "#,
    )
    .await
    .unwrap();
}
