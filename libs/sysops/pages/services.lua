local render = require("pages.render")
local form = require("pages.form")
local service_units = require("services.host.service_units")
local ctx = require("sysops.ctx")
local M = {}

local function pcall_or_empty(fn, ...)
  local ok, result = pcall(fn, ...)
  if ok and type(result) == "table" then return result end
  return {}
end

local function urlenc(s)
  return (tostring(s or "")):gsub("([^%w%-_%.~])", function(c)
    return string.format("%%%02X", string.byte(c))
  end)
end

local function services_url(args)
  args = args or {}
  return "/services"
    .. "?state=" .. urlenc(args.state or "all")
    .. "&type=" .. urlenc(args.type or "service")
    .. "&search=" .. urlenc(args.search or "")
    .. (args.ok and ("&ok=" .. urlenc(args.ok)) or "")
    .. (args.error and ("&error=" .. urlenc(args.error)) or "")
end

function M.page(req)
  local snap = ctx.state.snapshot()
  local q    = req.params or {}

  local state_filter = q.state  or "all"
  local type_filter  = q.type   or "service"
  local search       = (q.search or ""):lower()

  local glob = type_filter ~= "all" and ("*." .. type_filter) or nil
  local units = pcall_or_empty(systemd.list_units, glob)

  -- Count totals (over all services, not filtered)
  local all_units = glob and pcall_or_empty(systemd.list_units, nil) or units
  local total, active, failed, inactive = 0, 0, 0, 0
  local svc_n, timer_n, sock_n, mount_n = 0, 0, 0, 0
  for _, u in ipairs(all_units) do
    total = total + 1
    if u.active == "active"   then active   = active   + 1 end
    if u.active == "failed"   then failed   = failed   + 1 end
    if u.active == "inactive" then inactive = inactive + 1 end
    local name = u.name or ""
    if name:match("%.service$") then svc_n   = svc_n   + 1 end
    if name:match("%.timer$")   then timer_n = timer_n + 1 end
    if name:match("%.socket$")  then sock_n  = sock_n  + 1 end
    if name:match("%.mount$")   then mount_n = mount_n + 1 end
  end

  -- Apply state filter
  local filtered = {}
  for _, u in ipairs(units) do
    if state_filter == "all" or u.active == state_filter then
      -- Apply search filter
      local name = (u.name or ""):lower()
      local desc = (u.description or ""):lower()
      if search == "" or name:find(search, 1, true) or desc:find(search, 1, true) then
        filtered[#filtered + 1] = u
      end
    end
  end

  return render.render("services", {
    nav_active   = "services",
    host         = snap.host,
    machines     = snap.machines,
    units        = service_units.enrich(filtered),
    counts       = {
      total    = total,   active  = active,
      failed   = failed,  inactive = inactive,
      service  = svc_n,   timer   = timer_n,
      socket   = sock_n,  mount   = mount_n,
    },
    state_filter = state_filter,
    type_filter  = type_filter,
    search       = search,
    ok_msg       = q.ok,
    error_msg    = q.error,
  }, req)
end

local function lifecycle(req, action)
  local f = form.parse(req)
  local unit = f.unit or ""
  local state_filter = f.state or "all"
  local type_filter = f.type or "service"
  local search = f.search or ""

  local res = service_units.action(unit, action)
  if not res.ok then
    return {
      status = 303,
      headers = {
        ["Location"] = services_url({
          state = state_filter,
          type = type_filter,
          search = search,
          error = action .. " failed for " .. unit .. ": " .. (res.error or "?"),
        }),
      },
    }
  end

  return {
    status = 303,
    headers = {
      ["Location"] = services_url({
        state = state_filter,
        type = type_filter,
        search = search,
        ok = action .. " requested for " .. unit,
      }),
    },
  }
end

function M.restart(req)
  return lifecycle(req, "restart")
end

function M.start(req)
  return lifecycle(req, "start")
end

function M.stop(req)
  return lifecycle(req, "stop")
end

return M
