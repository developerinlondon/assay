-- /machines/<name>/services — units inside the nspawn container.
-- Source: shell-out to `systemctl --machine=<name> list-units --output=json`.
-- Tracked upstream at developerinlondon/assay#112 (add `machine` opt to
-- systemd.list_units / list_timers); swap to direct binding once that lands.
-- Process list reuses the cgroup walk via state.machine_deep().

local render = require("pages.render")
local state  = require("services.state")
local priv   = require("services.host.privilege")

local M = {}

local function find_machine(snap, name)
  for _, m in ipairs(snap.machines) do
    if m.name == name then return m end
  end
  return nil
end

-- nspawn machine names: alphanumeric plus -._ (matches systemd-machined validity).
local function valid_machine_name(s)
  return type(s) == "string" and #s > 0 and #s <= 64 and s:match("^[%w_%-%.]+$") ~= nil
end

local TYPE_WHITELIST = { all = true, service = true, timer = true, socket = true }

-- The --machine transport requires host-root. priv.elevated_prefix is
-- "" when knowhere runs as root and "sudo -n " otherwise (see
-- services/host/privilege.lua and deploy/knowhere-machinectl.sudoers.example).
local function fetch_units(name, type_filter)
  if not valid_machine_name(name) then return {}, "invalid machine name" end
  if not TYPE_WHITELIST[type_filter] then return {}, "invalid type filter" end
  local glob = (type_filter ~= "all") and (" --type=" .. type_filter) or ""
  local cmd = priv.elevated_prefix .. "systemctl --machine=" .. name
    .. " list-units --all --output=json --no-pager" .. glob
  local ok, r = pcall(shell.exec, cmd)
  if not ok or not r or r.status ~= 0 then
    return {}, (r and r.stderr) or "shell.exec failed"
  end
  local ok2, parsed = pcall(json.parse, r.stdout or "[]")
  if not ok2 or type(parsed) ~= "table" then return {}, "json parse failed" end
  return parsed, nil
end

local function fmt_rss(bytes)
  if bytes >= 1073741824 then return string.format("%.1f G", bytes / 1073741824)
  elseif bytes >= 1048576 then return string.format("%.0f M", bytes / 1048576)
  elseif bytes >= 1024    then return string.format("%.0f K", bytes / 1024)
  else                         return tostring(bytes) .. " B" end
end

local function enrich_processes(procs)
  local out = {}
  for _, p in ipairs(procs or {}) do
    out[#out + 1] = {
      pid = p.pid, cmdline = p.cmdline, state = p.state,
      cpu_pct = string.format("%.2f", p.cpu_pct or 0),
      rss_fmt = fmt_rss(p.rss_bytes or 0),
      threads = p.threads,
    }
  end
  return out
end

function M.page(req)
  local name = (req.path or ""):match("^/machines/([^/]+)/services$")
  if not name then return { status = 404, body = "not found" } end

  local snap = state.snapshot()
  local machine = find_machine(snap, name)
  if not machine then return { status = 404, body = "machine not found: " .. name } end

  local q = req.params or {}
  local state_filter = q.state  or "all"
  local type_filter  = q.type   or "service"
  local search       = (q.search or ""):lower()

  local raw_units, err = fetch_units(name, type_filter)

  -- systemctl --output=json keys: unit, load, active, sub, description.
  local total, active, failed, inactive = 0, 0, 0, 0
  local svc_n, timer_n, sock_n = 0, 0, 0
  for _, u in ipairs(raw_units) do
    total = total + 1
    if u.active == "active"   then active   = active   + 1 end
    if u.active == "failed"   then failed   = failed   + 1 end
    if u.active == "inactive" then inactive = inactive + 1 end
    local n = u.unit or ""
    if n:match("%.service$") then svc_n   = svc_n   + 1 end
    if n:match("%.timer$")   then timer_n = timer_n + 1 end
    if n:match("%.socket$")  then sock_n  = sock_n  + 1 end
  end

  local filtered = {}
  for _, u in ipairs(raw_units) do
    if state_filter == "all" or u.active == state_filter then
      local n = (u.unit or ""):lower()
      local d = (u.description or ""):lower()
      if search == "" or n:find(search, 1, true) or d:find(search, 1, true) then
        filtered[#filtered + 1] = {
          name        = u.unit,
          load        = u.load,
          active      = u.active,
          sub         = u.sub,
          description = u.description,
        }
      end
    end
  end

  local deep = state.machine_deep(name)

  return render.render("machines/services", {
    nav_active   = "machine:" .. name,
    page_title   = name .. " — services",
    machine_tab  = "services",
    host         = snap.host,
    machines     = snap.machines,
    machine      = machine,
    units        = filtered,
    counts       = {
      total = total, active = active, failed = failed, inactive = inactive,
      service = svc_n, timer = timer_n, socket = sock_n,
    },
    state_filter = state_filter,
    type_filter  = type_filter,
    search       = search,
    processes    = enrich_processes(deep.processes),
    fetch_error  = err,
  }, req)
end

return M
