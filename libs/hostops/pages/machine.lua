local render    = require("pages.render")
local resources = require("services.nspawn.resources")

local hostops_ctx = require("hostops.ctx")
local M = {}

local function fmt_rss(bytes)
  if bytes >= 1073741824 then
    return string.format("%.1f G", bytes / 1073741824)
  elseif bytes >= 1048576 then
    return string.format("%.0f M", bytes / 1048576)
  elseif bytes >= 1024 then
    return string.format("%.0f K", bytes / 1024)
  else
    return tostring(bytes) .. " B"
  end
end

local function enrich_processes(procs)
  local out = {}
  for _, p in ipairs(procs or {}) do
    out[#out + 1] = {
      pid      = p.pid,
      cmdline  = p.cmdline,
      state    = p.state,
      cpu_pct  = string.format("%.2f", p.cpu_pct or 0),
      rss_fmt  = fmt_rss(p.rss_bytes or 0),
      threads  = p.threads,
    }
  end
  return out
end

local function enrich_journal(entries)
  local out = {}
  for _, e in ipairs(entries or {}) do
    local msg = e.message or e.MESSAGE or ""
    local ts  = e.timestamp or e.__REALTIME_TIMESTAMP or ""
    local src = e.machine or e._MACHINE_ID or e.unit or e.SYSLOG_IDENTIFIER or ""

    local ts_fmt = ts
    if type(ts) == "number" then
      local secs = math.floor(ts / 1e6)
      local h = math.floor(secs / 3600) % 24
      local m = math.floor(secs / 60) % 60
      local s = secs % 60
      ts_fmt = string.format("%02d:%02d:%02d", h, m, s)
    elseif type(ts) == "string" and #ts > 10 then
      ts_fmt = ts:sub(1, 8)
    end

    local pri = tonumber(e.priority or e.PRIORITY or "6") or 6
    local line_class = ""
    if pri <= 3 then line_class = "err"
    elseif pri <= 4 then line_class = "warn"
    end

    out[#out + 1] = {
      ts        = ts_fmt,
      src       = tostring(src):sub(1, 24),
      msg       = tostring(msg),
      line_class = line_class,
    }
  end
  return out
end

local function slice(t, n)
  local out = {}
  for i = 1, math.min(n, #t) do out[i] = t[i] end
  return out
end

local function find_machine(snap, name)
  for _, m in ipairs(snap.machines) do
    if m.name == name then return m end
  end
  return nil
end

function M.detail(req)
  local path = (req.path or "")
  local name = path:match("^/machines/(.+)$")
  if not name or name == "" then
    return { status = 404, body = "not found" }
  end

  local snap    = hostops_ctx.state.snapshot()
  local machine = find_machine(snap, name)

  if not machine then
    return { status = 404, body = "machine not found: " .. name }
  end

  local deep    = hostops_ctx.state.machine_deep(name)
  local journal = enrich_journal(slice(deep.journal, 10))

  local ctx = {
    nav_active    = "machine:" .. name,
    machine_tab   = "overview",
    page_title    = name,
    host          = snap.host,
    machines      = snap.machines,
    machine       = machine,
    journal       = journal,
    nspawn_config = deep.nspawn_config,
  }
  ctx.machine_utilization = render.fragment("machine_utilization", ctx).body
  ctx.machine_journal     = render.fragment("machine_journal",     ctx).body
  ctx.machine_resources   = render.fragment("machine_resources_card", {
    machine_name = name,
    resources    = resources.read(name),
  }).body

  return render.render("machine", ctx, req)
end

-- Fragment handlers for SSE-triggered partial refresh.
function M.utilization(req)
  local name = (req.path or ""):match("^/api/machines/([^/]+)/utilization$")
  if not name then return { status = 404, body = "not found" } end
  local snap    = hostops_ctx.state.snapshot()
  local machine = find_machine(snap, name)
  if not machine then return { status = 404, body = "machine not found" } end
  return render.fragment("machine_utilization", { machine = machine })
end

function M.processes(req)
  local name = (req.path or ""):match("^/api/machines/([^/]+)/processes$")
  if not name then return { status = 404, body = "not found" } end
  local deep = hostops_ctx.state.machine_deep(name)
  return render.fragment("machine_processes", { processes = enrich_processes(deep.processes) })
end

function M.journal(req)
  local name = (req.path or ""):match("^/api/machines/([^/]+)/journal$")
  if not name then return { status = 404, body = "not found" } end
  local deep = hostops_ctx.state.machine_deep(name)
  return render.fragment("machine_journal", { journal = enrich_journal(deep.journal) })
end

return M
