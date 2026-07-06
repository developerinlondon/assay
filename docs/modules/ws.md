---
category: Builtins
---

## ws

WebSocket client. No `require()` needed.

- `ws.connect(url, opts?)` → conn — Connect to a WebSocket server. `opts` is optional;
  `ws.connect(url)` alone is unchanged.
- `ws.send(conn, msg)` → nil — Send a text frame
- `ws.send_binary(conn, bytes)` → nil — Send a binary frame (payload is a binary-safe Lua string of
  arbitrary bytes)
- `ws.recv(conn)` → string — Receive the next message (blocking). Text frames come back as their
  UTF-8 bytes; binary frames come back as raw bytes (Lua strings are binary-safe), so non-UTF-8
  payloads no longer error
- `ws.protocol(conn)` → string|nil — The subprotocol the server negotiated during the handshake, or
  nil if none
- `ws.close(conn)` → nil — Close connection

### Connect options

`opts` is a table; every field is optional:

- `subprotocols` — array of strings sent as the `Sec-WebSocket-Protocol` handshake header. The
  server picks one; read it back with `ws.protocol(conn)`.
- `headers` — map of `string → string` added as extra handshake request headers (e.g.
  `Authorization`).
- `insecure` — bool. When true, the TLS handshake skips server certificate verification (for
  `wss://` against a server whose CA the runtime does not trust). Ignored for plain `ws://`.
  Defaults to false.

```lua
-- Backward compatible: no opts.
local conn = ws.connect("ws://example.com/socket")
ws.send(conn, "hello")
print(ws.recv(conn))
ws.close(conn)

-- Subprotocols + bearer auth + skip TLS verification.
local conn = ws.connect("wss://example.com/api/exec", {
  subprotocols = { "v4.channel.k8s.io" },
  headers = { Authorization = "Bearer " .. token },
  insecure = true,
})
print(ws.protocol(conn))  -- "v4.channel.k8s.io"

-- Binary frames round-trip as raw bytes (channel-prefixed protocols, etc.).
ws.send_binary(conn, "\0raw\255bytes")
local frame = ws.recv(conn)
```

### Gating

`ws.connect` is a gated (mutating) builtin: read-only mode replaces it with a blocking stub and
approval mode suspends it for confirmation. Anything built on top of it — including `k8s.pods:exec`
— inherits that gating. `ws.send`, `ws.send_binary`, `ws.recv`, `ws.protocol` and `ws.close` operate
on an already-established connection and are not separately gated.
