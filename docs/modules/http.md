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
  - Handlers receive `{method, path, body, headers, query}`, return `{status, body, json?, headers?}`
  - Handlers can call async builtins (`http.get`, `sleep`, etc.)
  - Header values can be a string or an array of strings. Array values emit the same header
    name multiple times — required for `Set-Cookie` with multiple cookies, and useful for
    `Link`, `Vary`, `Cache-Control`, etc.:
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
