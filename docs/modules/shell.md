---
category: Stdlib
---

## assay.shell

WebSocket ↔ PTY bridge for hosting in-browser terminals (xterm.js etc.) from inside an `http.serve`
handler. Combines `process.spawn_pty` and the new `{ws = ...}` server-upgrade in `http.serve` into a
single 2-line bridge call.

```lua
local shell = require("assay.shell")
```

### `shell.bridge(conn, opts)`

Bridge a WebSocket server connection to a fresh PTY child until either side closes. Returns when the
child exits or the peer disconnects.

| Field       | Type     | Default | Notes                                   |
| ----------- | -------- | ------- | --------------------------------------- |
| `opts.cmd`  | string   | —       | Required. argv[0]: binary or PATH name. |
| `opts.args` | string[] | `{}`    | argv[1..].                              |
| `opts.cwd`  | string   | inherit | Child working directory.                |
| `opts.env`  | table    | inherit | Extra env vars (key=value).             |
| `opts.cols` | integer  | `80`    | Initial PTY columns.                    |
| `opts.rows` | integer  | `24`    | Initial PTY rows.                       |

### Wire format with the browser

- **Binary frames** from the browser → forwarded raw to the PTY's stdin.
- **Text frames** matching `{"resize":{"cols":N,"rows":M}}` → trigger `pty:resize(N, M)`. Anything
  else (plain text input) is forwarded raw.
- **PTY output** → sent back as binary frames.

Resize is best-effort: malformed JSON, missing keys, or non-positive dimensions are ignored
silently.

### Example

A complete browser shell endpoint:

```lua
local shell = require("assay.shell")

http.serve(8080, {
  GET = {
    ["/shell"] = function(req)
      return {
        ws = function(conn)
          shell.bridge(conn, {
            cmd  = "bash",
            args = { "-l" },
            env  = { TERM = "xterm-256color" },
          })
        end,
      }
    end,
  },
})
```

Pair with `xterm.js` on the browser side; see `examples/shell-server.lua` for a runnable end-to-end
demo with HTML and the resize protocol wired up.

### Caveats

- `http.serve` is plaintext. Put a reverse proxy in front for `wss://`.
- The bridge does **not** do authentication; gate the upgrade in your handler before calling
  `shell.bridge` (e.g., check a session cookie or API key).
- Linux + macOS only — `process.spawn_pty` is unavailable elsewhere.
