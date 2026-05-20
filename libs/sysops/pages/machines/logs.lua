-- /machines/<name>/logs — full journal stream for the container.
-- Reuses the host journal binding with `machine = name`. journalctl's
-- `--machine=` switch requires host-root, so we pass `elevate = true`
-- when sysops runs unprivileged — the assay binding wraps the
-- subprocess in `sudo -n` and the NOPASSWD allowlist
-- (deploy/sysops-machinectl.sudoers.template) covers
-- `journalctl --machine=*`. The SSE stream at /api/logs/stream
-- already accepts ?machine=<name>; the template plumbs the machine via
-- window.SYSOPS_LOG_MACHINE.

local render = require("pages.render")
local priv   = require("services.host.privilege")

local ctx = require("sysops.ctx")
local M = {}

local function find_machine(snap, name)
  for _, m in ipairs(snap.machines) do
    if m.name == name then return m end
  end
  return nil
end

local function fmt_ts(us)
  if not us or us == 0 then return "" end
  local s = math.floor(us / 1000000)
  return os.date("%H:%M:%S", s)
end

local function fetch_journal(name)
  local opts = { machine = name, lines = 100, elevate = not priv.is_root }
  local ok, r = pcall(systemd.journal, opts)
  if not ok then
    return {}, tostring(r)
  end
  if type(r) ~= "table" then
    return {}, "systemd.journal returned non-table"
  end
  return r, nil
end

function M.page(req)
  local name = (req.path or ""):match("^/machines/([^/]+)/logs$")
  if not name then return { status = 404, body = "not found" } end

  local snap = ctx.state.snapshot()
  local machine = find_machine(snap, name)
  if not machine then return { status = 404, body = "machine not found: " .. name } end

  local initial, err = fetch_journal(name)
  for _, e in ipairs(initial) do e.ts_pretty = fmt_ts(e.ts) end

  return render.render("machines/logs", {
    nav_active  = "machine:" .. name,
    page_title  = name .. " — logs",
    machine_tab = "logs",
    host        = snap.host,
    machines    = snap.machines,
    machine     = machine,
    initial     = initial,
    fetch_error = err,
  }, req)
end

return M
