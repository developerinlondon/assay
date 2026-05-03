local M = {}

local function trim(s)
  return s:match("^%s*(.-)%s*$")
end

-- Parse a simple INI-style file into { [section] = { key = value | [values] } }
local function parse_ini(text)
  local result = {}
  local section = nil
  for raw_line in (text .. "\n"):gmatch("([^\n]*)\n") do
    local line = trim(raw_line)
    if line == "" or line:sub(1, 1) == "#" or line:sub(1, 1) == ";" then
      -- skip
    elseif line:sub(1, 1) == "[" then
      section = line:match("^%[(.-)%]")
      if section then
        section = section:lower()
        result[section] = result[section] or {}
      end
    elseif section then
      local k, v = line:match("^([^=]+)=(.*)$")
      if k then
        k = trim(k):lower()
        v = trim(v)
        local cur = result[section][k]
        if cur == nil then
          result[section][k] = v
        elseif type(cur) == "table" then
          cur[#cur + 1] = v
        else
          result[section][k] = { cur, v }
        end
      end
    end
  end
  return result
end

local function as_list(v)
  if v == nil then return {} end
  if type(v) == "table" then return v end
  return { v }
end

local function parse_bind(v, ro)
  local host, guest = v:match("^([^:]+):(.+)$")
  if host then
    return { host = host, guest = guest, ro = ro }
  else
    return { host = v, guest = v, ro = ro }
  end
end

local function merge_ini(base, overlay)
  for section, kvs in pairs(overlay) do
    if not base[section] then base[section] = {} end
    for k, v in pairs(kvs) do
      local cur = base[section][k]
      if cur == nil then
        base[section][k] = v
      elseif type(v) == "table" then
        if type(cur) == "table" then
          for _, item in ipairs(v) do cur[#cur + 1] = item end
        else
          base[section][k] = { cur }
          for _, item in ipairs(v) do base[section][k][#base[section][k] + 1] = item end
        end
      else
        base[section][k] = v
      end
    end
  end
end

function M.read(name)
  local main_path = "/etc/systemd/nspawn/" .. name .. ".nspawn"
  local ok_read, text = pcall(fs.read, main_path)
  if not ok_read or not text then return nil end

  local ini = parse_ini(text)

  -- Merge drop-ins from /etc/systemd/nspawn/<name>.nspawn.d/*.conf
  local dropin_dir = main_path .. ".d"
  local dropin_names = {"override.conf", "10-override.conf", "20-override.conf",
                        "local.conf", "extra.conf", "custom.conf"}
  for _, fname in ipairs(dropin_names) do
    local ok_d, dt = pcall(fs.read, dropin_dir .. "/" .. fname)
    if ok_d and dt then merge_ini(ini, parse_ini(dt)) end
  end

  local exec_sec  = ini["exec"]  or {}
  local files_sec = ini["files"] or {}
  local net_sec   = ini["network"] or {}

  -- Parse bind mounts
  local binds = {}
  for _, v in ipairs(as_list(files_sec["bind"])) do
    binds[#binds + 1] = parse_bind(v, false)
  end
  for _, v in ipairs(as_list(files_sec["bindreadonly"])) do
    binds[#binds + 1] = parse_bind(v, true)
  end

  local inaccessible = as_list(files_sec["inaccessible"])

  -- Parse service drop-ins for resource limits
  local resources = {}
  local svc_dropin_dir = "/etc/systemd/system/systemd-nspawn@" .. name .. ".service.d"
  for _, fname in ipairs({"override.conf", "10-override.conf", "local.conf", "resources.conf"}) do
    local ok_t, t = pcall(fs.read, svc_dropin_dir .. "/" .. fname)
    if ok_t and t then
      local svc_ini = parse_ini(t)
      local svc = svc_ini["service"] or {}
      if svc["memorymax"]  then resources.memory_max  = svc["memorymax"]  end
      if svc["cpuquota"]   then resources.cpu_quota   = svc["cpuquota"]   end
      if svc["tasksmax"]   then resources.tasks_max   = svc["tasksmax"]   end
    end
  end

  return {
    exec = {
      boot          = exec_sec["boot"],
      notify_ready  = exec_sec["notifyready"],
      private_users = exec_sec["privateusers"],
      capability    = exec_sec["capability"],
      syscall_filter = exec_sec["syscallfilter"],
    },
    files = {
      bind         = binds,
      inaccessible = inaccessible,
    },
    network = {
      private_users    = net_sec["privateusers"],
      virtual_ethernet = net_sec["virtualethernet"],
      bridge           = net_sec["bridge"],
      zone             = net_sec["zone"],
    },
    resources = resources,
  }
end

return M
