---
category: Builtins
---

## http

HTTP client and server. No `require()` needed. All responses return `{status, body, headers}`.
Options table supports `{headers = {["X-Key"] = "value"}}`.

- `http.get(url, opts?)` → `{status, body, headers}` — GET request
- `http.post(url, body, opts?)` → `{status, body, headers}` — POST request (auto-JSON if table body)
- `http.put(url, body, opts?)` → `{status, body, headers}` — PUT request
- `http.patch(url, body, opts?)` → `{status, body, headers}` — PATCH request
- `http.delete(url, opts?)` → `{status, body, headers}` — DELETE request
- `http.serve(port, routes)` → blocks — Start HTTP server with async handlers
  - Routes: `{GET = {["/path"] = function(req) return {status=200, body="ok"} end}}`
  - Handlers receive `{method, path, body, headers, query}`, return
    `{status, body, json?, headers?}`
  - Handlers can call async builtins (`http.get`, `sleep`, etc.)
  - Header values can be a string or an array of strings. Array values emit the same header name
    multiple times — required for `Set-Cookie` with multiple cookies, and useful for `Link`, `Vary`,
    `Cache-Control`, etc.:
    ```lua
    return {
      status = 200,
      headers = {
        ["Set-Cookie"] = {
          "session=abc; Path=/; HttpOnly",
          "csrf=xyz; Path=/",
        },
      },
      body = "ok",
    }
    ```
  - SSE: return `{sse = function(send) send({event="update", data="hello", id="1"}) end}`
    - `send()` accepts: `event`, `data`, `id`, `retry` fields
    - Sets `Content-Type: text/event-stream` automatically
    - Stream closes when function returns
  - WebSocket upgrade (v0.15.1+): return `{ws = function(conn) ... end}` from any handler whose
    request carries `Upgrade: websocket`. `http.serve` validates the handshake, sends
    `101 Switching Protocols`, and hands the upgraded connection to the callback:
    ```lua
    GET = {
      ["/echo"] = function(req)
        return {
          ws = function(conn)
            while true do
              local msg = conn:read()           -- string, nil on close
              if not msg then break end
              conn:write(msg)                   -- text frame
              -- conn:write(bytes, { binary = true }) for binary frames
            end
          end,
          headers = { ["X-Shell-Allowed"] = "true" }, -- optional
        }
      end,
    }
    ```
    Connection methods: `conn:read()` (blocks until next text/binary frame, returns nil on close),
    `conn:write(data, opts?)` (`opts.binary=true` for binary frames), `conn:close(code?, reason?)`,
    `conn:is_closed()`. Field: `conn.peer_addr`. Ping/pong is handled automatically by the
    underlying tungstenite stack. For browser-shell bridging see [`assay.shell`](shell.md).
