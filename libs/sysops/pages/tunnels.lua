local render = require("pages.render")
local ctx = require("sysops.ctx")
local M = {}

local function pcall_or_nil(fn, ...)
  local ok, result = pcall(fn, ...)
  if ok then return result end
  return nil
end

-- Minimal YAML parser for the cloudflared config shape:
--   tunnel: <id>
--   ingress:
--     - hostname: foo.example.com
--       service: http://...
--     - service: http_status:404
local function read_config_yml()
  local body = pcall_or_nil(fs.read, "/etc/cloudflared/config.yml")
  if not body then return { tunnel = nil, ingress = {} } end

  local tunnel_id = body:match("^tunnel:%s*([%w%-]+)") or body:match("\ntunnel:%s*([%w%-]+)")
  local ingress = {}

  local in_ingress = false
  local current = nil
  for line in (body .. "\n"):gmatch("([^\n]*)\n") do
    if line:match("^ingress:%s*$") then
      in_ingress = true
    elseif in_ingress then
      -- `  - hostname: foo` starts a new entry
      local hostname = line:match("^%s+%-%s+hostname:%s*(.+)$")
      -- `    service: http://...` belongs to current entry
      local service_cont = line:match("^%s+service:%s*(.+)$")
      -- `  - service: http_status:404` is a catch-all entry
      local entry_service = line:match("^%s+%-%s+service:%s*(.+)$")

      if hostname then
        if current then ingress[#ingress + 1] = current end
        current = { hostname = hostname:gsub("%s+$", ""), service = nil }
      elseif service_cont and current and not current.service then
        current.service = service_cont:gsub("%s+$", "")
      elseif entry_service then
        if current then ingress[#ingress + 1] = current end
        current = nil
        ingress[#ingress + 1] = { hostname = nil, service = entry_service:gsub("%s+$", "") }
      elseif not line:match("^%s") and not line:match("^%-%-") and line ~= "" then
        -- new top-level key — end of ingress block
        if current then ingress[#ingress + 1] = current end
        current = nil
        in_ingress = false
      end
    end
  end
  if current then ingress[#ingress + 1] = current end

  return { tunnel = tunnel_id, ingress = ingress }
end

-- Format microsecond timestamp from systemd into readable string
local function fmt_since(us)
  if not us or us == 0 then return nil end
  local secs = math.floor(us / 1000000)
  local t = os.date("*t", secs)
  return string.format("%04d-%02d-%02d %02d:%02d:%02d", t.year, t.month, t.day, t.hour, t.min, t.sec)
end

function M.page(req)
  local snap = ctx.state.snapshot()

  local cfd_raw = pcall_or_nil(systemd.unit_status, "cloudflared.service") or {}
  local cfd = {
    active      = cfd_raw.active or "unknown",
    sub         = cfd_raw.sub,
    load        = cfd_raw.load,
    description = cfd_raw.description,
    main_pid    = cfd_raw.main_pid,
    since       = fmt_since(cfd_raw.since),
  }

  local config = read_config_yml()

  -- Probe each ingress rule that has an http(s) service URL
  local probes = {}
  for _, rule in ipairs(config.ingress or {}) do
    if rule.hostname and rule.service and rule.service:find("^http") then
      local ok, r = pcall(http.get, rule.service, { timeout_ms = 1500 })
      probes[#probes + 1] = {
        hostname = rule.hostname,
        service  = rule.service,
        probe_ok = ok and r ~= nil,
        status   = ok and r and r.status,
        latency  = ok and r and r.latency_ms,
      }
    end
  end

  -- Sort: failures first, then by status code ascending
  table.sort(probes, function(a, b)
    local a_fail = (not a.probe_ok) and 1 or 0
    local b_fail = (not b.probe_ok) and 1 or 0
    if a_fail ~= b_fail then return a_fail > b_fail end
    return (a.status or 999) < (b.status or 999)
  end)

  return render.render("tunnels", {
    nav_active     = "tunnels",
    host           = snap.host,
    machines       = snap.machines,
    cfd            = cfd,
    tunnel_id      = config.tunnel,
    probes         = probes,
    config_present = config.tunnel ~= nil,
    route_count    = #(config.ingress or {}),
  }, req)
end

return M
