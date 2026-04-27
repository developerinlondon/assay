-- examples/shell-server.lua
--
-- Minimal in-browser shell server. Run with:
--
--   assay run examples/shell-server.lua
--
-- Then open http://localhost:8080/ in a browser. Each visitor gets their own
-- shell process attached to a real PTY, with terminal resize support via a
-- JSON control frame on the WebSocket.
--
-- For a real deployment you'd put a reverse proxy in front for `wss://` and
-- gate the upgrade behind authentication.

local shell = require("assay.shell")

local INDEX_HTML = [[
<!doctype html>
<html><head>
  <meta charset="utf-8">
  <title>assay shell</title>
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/xterm@5.3.0/css/xterm.css">
  <script src="https://cdn.jsdelivr.net/npm/xterm@5.3.0/lib/xterm.js"></script>
  <script src="https://cdn.jsdelivr.net/npm/xterm-addon-fit@0.8.0/lib/xterm-addon-fit.js"></script>
  <style>html,body,#term{width:100%;height:100%;margin:0;background:#000}</style>
</head><body>
<div id="term"></div>
<script>
  const term = new Terminal({ convertEol: true });
  const fit = new FitAddon.FitAddon();
  term.loadAddon(fit);
  term.open(document.getElementById("term"));
  fit.fit();

  const ws = new WebSocket(`ws://${location.host}/shell`);
  ws.binaryType = "arraybuffer";
  ws.onopen = () => {
    ws.send(JSON.stringify({ resize: { cols: term.cols, rows: term.rows } }));
    term.onData(d => ws.send(d));
    window.addEventListener("resize", () => {
      fit.fit();
      ws.send(JSON.stringify({ resize: { cols: term.cols, rows: term.rows } }));
    });
  };
  ws.onmessage = ev => {
    if (ev.data instanceof ArrayBuffer) {
      term.write(new Uint8Array(ev.data));
    } else {
      term.write(ev.data);
    }
  };
  ws.onclose = () => term.write("\r\n[connection closed]\r\n");
</script>
</body></html>
]]

http.serve(8080, {
  GET = {
    ["/"] = function(req)
      return {
        status = 200,
        body = INDEX_HTML,
        headers = { ["Content-Type"] = "text/html; charset=utf-8" },
      }
    end,
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
