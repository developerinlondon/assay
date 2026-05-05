local render = require("pages.render")
local ctx = require("sysops.ctx")
local M = {}

function M.machine_page(req)
  local name = (req.path or ""):match("^/machines/([^/]+)/shell$")
  if not name then return { status = 404, body = "not found" } end

  local snap = ctx.state.snapshot()
  local machine
  for _, m in ipairs(snap.machines) do
    if m.name == name then machine = m; break end
  end
  if not machine then
    return { status = 404, body = "machine not found: " .. name }
  end

  return render.render("shell", {
    nav_active  = "machine:" .. name,
    machine_tab = "shell",
    host        = snap.host,
    machines    = snap.machines,
    machine     = machine,
    target      = name,
    back_url    = "/machines/" .. name,
    ws_url      = "/api/machines/" .. name .. "/shell",
  }, req)
end

function M.host_page(req)
  local snap = ctx.state.snapshot()
  return render.render("shell", {
    nav_active = "host_shell",
    host       = snap.host,
    machines   = snap.machines,
    target     = (snap.host and snap.host.name or "host"),
    back_url   = "/",
    ws_url     = "/api/host/shell",
  }, req)
end

return M
