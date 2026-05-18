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
    .. (args.sort and args.sort ~= "" and ("&sort=" .. urlenc(args.sort) .. "&dir=" .. urlenc(args.dir or "desc")) or "")
    .. (args.ok and ("&ok=" .. urlenc(args.ok)) or "")
    .. (args.error and ("&error=" .. urlenc(args.error)) or "")
end

local function clean_sort_key(v)
  v = tostring(v or "")
  if v == "memory" or v == "cpu" then return v end
  return ""
end

local function clean_sort_dir(v)
  if tostring(v or "") == "asc" then return "asc" end
  return "desc"
end

local function next_sort_dir(key, sort_key, sort_dir)
  if sort_key == key and sort_dir == "desc" then return "asc" end
  return "desc"
end

local function sort_value(row, sort_key)
  if sort_key == "memory" then return row.memory_sort end
  if sort_key == "cpu" then return row.cpu_sort end
  return nil
end

local function sort_units(units, sort_key, sort_dir)
  if sort_key == "" then return units end

  table.sort(units, function(a, b)
    local av = sort_value(a, sort_key)
    local bv = sort_value(b, sort_key)
    local an = tostring(a.name or a.unit or "")
    local bn = tostring(b.name or b.unit or "")

    if av == nil and bv == nil then return an < bn end
    if av == nil then return false end
    if bv == nil then return true end
    if av == bv then return an < bn end
    if sort_dir == "asc" then return av < bv end
    return av > bv
  end)
  return units
end

function M.page(req)
  local snap = ctx.state.snapshot()
  local q    = req.params or {}

  local state_filter = q.state  or "all"
  local type_filter  = q.type   or "service"
  local search       = (q.search or ""):lower()
  local sort_key     = clean_sort_key(q.sort)
  local sort_dir     = clean_sort_dir(q.dir)

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

  local enriched = sort_units(service_units.enrich(filtered), sort_key, sort_dir)

  return render.render("services", {
    nav_active   = "services",
    host         = snap.host,
    machines     = snap.machines,
    units        = enriched,
    counts       = {
      total    = total,   active  = active,
      failed   = failed,  inactive = inactive,
      service  = svc_n,   timer   = timer_n,
      socket   = sock_n,  mount   = mount_n,
    },
    state_filter = state_filter,
    type_filter  = type_filter,
    search       = search,
    sort_key     = sort_key,
    sort_dir     = sort_dir,
    state_all_url = services_url({
      state = "all", type = type_filter, search = search,
      sort = sort_key, dir = sort_dir,
    }),
    state_active_url = services_url({
      state = "active", type = type_filter, search = search,
      sort = sort_key, dir = sort_dir,
    }),
    state_failed_url = services_url({
      state = "failed", type = type_filter, search = search,
      sort = sort_key, dir = sort_dir,
    }),
    state_inactive_url = services_url({
      state = "inactive", type = type_filter, search = search,
      sort = sort_key, dir = sort_dir,
    }),
    type_service_url = services_url({
      state = state_filter, type = "service", search = search,
      sort = sort_key, dir = sort_dir,
    }),
    type_timer_url = services_url({
      state = state_filter, type = "timer", search = search,
      sort = sort_key, dir = sort_dir,
    }),
    type_socket_url = services_url({
      state = state_filter, type = "socket", search = search,
      sort = sort_key, dir = sort_dir,
    }),
    type_all_url = services_url({
      state = state_filter, type = "all", search = search,
      sort = sort_key, dir = sort_dir,
    }),
    memory_sort_url = services_url({
      state = state_filter, type = type_filter, search = search,
      sort = "memory", dir = next_sort_dir("memory", sort_key, sort_dir),
    }),
    cpu_sort_url = services_url({
      state = state_filter, type = type_filter, search = search,
      sort = "cpu", dir = next_sort_dir("cpu", sort_key, sort_dir),
    }),
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
  local sort_key = clean_sort_key(f.sort)
  local sort_dir = clean_sort_dir(f.dir)

  local res = service_units.action(unit, action)
  if not res.ok then
    return {
      status = 303,
      headers = {
        ["Location"] = services_url({
          state = state_filter,
          type = type_filter,
          search = search,
          sort = sort_key,
          dir = sort_dir,
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
        sort = sort_key,
        dir = sort_dir,
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
