local shell = require("assay.shell")
local ctx = require("hostops.ctx")
local M = {}

local function actor_from(req)
  local h = req.headers or {}
  return h["Cf-Access-Authenticated-User-Email"]
      or h["cf-access-authenticated-user-email"]
      or "local-dev"
end

local function is_ws_upgrade(req)
  local h = req.headers or {}
  local up = (h.Upgrade or h.upgrade or ""):lower()
  return up:find("websocket", 1, true) ~= nil
end

-- Default env for PTY-spawned shells. Without TERM the spawned bash
-- thinks it's on a dumb terminal and skips colored prompt / `ls
-- --color=auto` etc., which makes the in-browser shell look unstyled
-- compared to a real ssh session.
local PTY_ENV = {
  TERM     = "xterm-256color",
  COLORTERM = "truecolor",
  LANG     = "C.UTF-8",
}

local function bridge(conn, opts, action, target, actor)
  opts.env = opts.env or PTY_ENV
  pcall(ctx.audit.append, { actor = actor, action = action .. ".opened", target = target, result = "ok" })
  shell.bridge(conn, opts)
  pcall(ctx.audit.append, { actor = actor, action = action .. ".closed", target = target, result = "ok" })
end

function M.handle_machine(req)
  local name = (req.path or ""):match("^/api/machines/([^/]+)/shell$")
  if not name then return { status = 404, body = "not found" } end
  if not is_ws_upgrade(req) then return { status = 426, body = "websocket upgrade required" } end

  local actor = actor_from(req)
  return {
    ws = function(conn)
      bridge(conn, {
        cmd  = "machinectl",
        args = { "shell", name },
        cols = 120, rows = 30,
      }, "machine.shell", name, actor)
    end,
  }
end

function M.handle_host(req)
  if not is_ws_upgrade(req) then return { status = 426, body = "websocket upgrade required" } end

  local actor = actor_from(req)
  return {
    ws = function(conn)
      bridge(conn, {
        cmd  = "/bin/bash",
        args = { "-l", "-i" },
        cols = 120, rows = 30,
      }, "host.shell", "host", actor)
    end,
  }
end

return M
