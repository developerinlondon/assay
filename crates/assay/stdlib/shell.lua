--- @module assay.shell
--- @description WebSocket ↔ PTY bridge for in-browser terminals (xterm.js, etc). Spawns a child on a fresh PTY, pipes raw bytes both directions, and interprets a small JSON resize control protocol on text frames.
--- @keywords shell, pty, websocket, terminal, xterm, browser, bridge
--- @quickref M.bridge(conn, opts) | Bridge a ws server conn to a `process.spawn_pty` child until either side closes

local M = {}

--- Bridge a WebSocket server connection to a fresh PTY child until either side closes.
---
--- @param conn  ws server connection (the `conn` argument to an `http.serve` `ws =` handler)
--- @param opts  table: { cmd=string, args?={...}, cwd?=string, env?={...}, cols?=int, rows?=int }
---
--- Wire format the bridge speaks with the browser:
---   - All binary frames are forwarded raw to the PTY's stdin.
---   - Text frames matching `{"resize":{"cols":N,"rows":M}}` trigger `pty:resize(N, M)`;
---     anything else is forwarded raw (so a plain text terminal client still works).
---   - PTY output goes back as binary frames.
---
--- Returns when either the PTY child exits or the websocket peer closes.
function M.bridge(conn, opts)
  if type(conn) ~= "userdata" then
    error("assay.shell.bridge: conn must be a ws server connection")
  end
  if type(opts) ~= "table" or type(opts.cmd) ~= "string" then
    error("assay.shell.bridge: opts.cmd (string) is required")
  end

  local pty = process.spawn_pty({
    cmd  = opts.cmd,
    args = opts.args,
    cwd  = opts.cwd,
    env  = opts.env,
    cols = opts.cols or 80,
    rows = opts.rows or 24,
  })

  -- PTY → WebSocket
  local pty_to_ws = async.spawn(function()
    while true do
      local chunk = pty:read()
      if chunk == nil then break end
      local ok = pcall(function() conn:write(chunk, { binary = true }) end)
      if not ok then break end
    end
    pcall(function() conn:close() end)
  end)

  -- WebSocket → PTY (with resize control-message detection)
  local ws_to_pty = async.spawn(function()
    while true do
      local msg = conn:read()
      if msg == nil then break end

      -- Resize control message: {"resize":{"cols":N,"rows":M}}.
      -- Only attempt JSON decode on payloads that look like one; anything
      -- else is forwarded raw so terminals that send normal text still work.
      if msg:sub(1, 1) == "{" and msg:find('"resize"', 1, true) then
        local ok, decoded = pcall(json.parse, msg)
        if ok and type(decoded) == "table" and type(decoded.resize) == "table" then
          local cols = tonumber(decoded.resize.cols)
          local rows = tonumber(decoded.resize.rows)
          if cols and rows and cols > 0 and rows > 0 then
            pcall(function() pty:resize(cols, rows) end)
            goto continue
          end
        end
      end

      pcall(function() pty:write(msg) end)
      ::continue::
    end
    pcall(function() pty:close() end)
  end)

  pty_to_ws:await()
  ws_to_pty:await()
end

return M
