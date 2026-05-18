--! Host systemd service-unit helpers for sysops pages.

local M = {}

local UNIT_SAFE = "^[%w%._%-%:@\\]+$"
local ALLOWED_ACTIONS = {
  restart = true,
  start = true,
  stop = true,
}
local INFO_FIELDS = {
  "memory_current",
  "tasks_current",
  "cpu_usage_nsec",
  "n_restarts",
  "unit_file_state",
  "fragment_path",
  "drop_in_paths",
  "main_pid",
  "exec_main_pid",
  "exec_main_status",
  "exec_start",
  "restart",
  "restart_usec",
  "user",
  "group",
  "working_directory",
  "memory_accounting",
  "cpu_accounting",
  "tasks_accounting",
}
local SHOW_PROPERTIES = {
  "MemoryCurrent",
  "TasksCurrent",
  "CPUUsageNSec",
  "NRestarts",
  "UnitFileState",
  "FragmentPath",
  "DropInPaths",
  "MainPID",
  "ExecMainPID",
  "ExecMainStatus",
  "ExecStart",
  "Restart",
  "RestartUSec",
  "User",
  "Group",
  "WorkingDirectory",
  "MemoryAccounting",
  "CPUAccounting",
  "TasksAccounting",
}

local function trim(s)
  return (tostring(s or "")):gsub("^%s+", ""):gsub("%s+$", "")
end

local function is_unset(v)
  return v == nil
      or v == ""
      or v == "[not set]"
      or v == "infinity"
      or v == "18446744073709551615"
end

local function number_value(v)
  if is_unset(v) then return nil end
  if type(v) == "number" then return v end
  return tonumber(tostring(v))
end

local function pick(t, names)
  if type(t) ~= "table" then return nil end
  for _, name in ipairs(names) do
    local v = t[name]
    if not is_unset(v) then return v end
  end
  return nil
end

local function shell_quote(s)
  return "'" .. tostring(s):gsub("'", "'\\''") .. "'"
end

local function dom_id(unit)
  return "svc-" .. tostring(unit or ""):gsub("[^%w_%-]", "-")
end

local function append_detail(details, label, value, opts)
  opts = opts or {}
  if is_unset(value) then return end
  if opts.skip_zero and tostring(value) == "0" then return end

  local s = trim(value)
  if s == "" then return end
  details[#details + 1] = { label = label, value = s }
end

local function detail_rows(row, info)
  local details = {}
  append_detail(details, "Load state", row.load)
  append_detail(details, "Active state", row.active)
  append_detail(details, "Sub state", row.sub)
  append_detail(details, "Unit file", info.unit_file_state)
  append_detail(details, "Fragment path", info.fragment_path)
  append_detail(details, "Drop-ins", info.drop_in_paths)
  append_detail(details, "Main PID", info.main_pid or info.exec_main_pid, { skip_zero = true })
  append_detail(details, "Exit status", info.exec_main_status)
  append_detail(details, "User", info.user)
  append_detail(details, "Group", info.group)
  append_detail(details, "Working directory", info.working_directory)
  append_detail(details, "Exec start", info.exec_start)
  append_detail(details, "Restart policy", info.restart)
  append_detail(details, "Restart delay", info.restart_usec)
  append_detail(details, "Memory accounting", info.memory_accounting)
  append_detail(details, "CPU accounting", info.cpu_accounting)
  append_detail(details, "Tasks accounting", info.tasks_accounting)
  return details
end

function M.valid_service_name(unit)
  return type(unit) == "string"
      and #unit > 8
      and not unit:match("^%-")
      and unit:match("%.service$") ~= nil
      and unit:match(UNIT_SAFE) ~= nil
end

function M.fmt_bytes(bytes)
  bytes = number_value(bytes)
  if not bytes then return "—" end
  if bytes >= 1073741824 then return string.format("%.1f G", bytes / 1073741824) end
  if bytes >= 1048576 then return string.format("%.0f M", bytes / 1048576) end
  if bytes >= 1024 then return string.format("%.0f K", bytes / 1024) end
  return tostring(math.floor(bytes)) .. " B"
end

function M.fmt_cpu_time(nsec)
  nsec = number_value(nsec)
  if not nsec then return "—" end
  if nsec == 0 then return "0s" end

  local secs = nsec / 1000000000
  if secs < 60 then
    local s = string.format("%.2f", secs):gsub("0+$", ""):gsub("%.$", "")
    return s .. "s"
  end
  if secs < 3600 then
    local mins = math.floor(secs / 60)
    local rem = math.floor(secs % 60)
    return string.format("%dm %ds", mins, rem)
  end
  local hours = math.floor(secs / 3600)
  local mins = math.floor((secs % 3600) / 60)
  return string.format("%dh %dm", hours, mins)
end

local function stats_from_unit_status(unit)
  if type(systemd) ~= "table" or type(systemd.unit_status) ~= "function" then
    return {}
  end
  local ok, status = pcall(systemd.unit_status, unit)
  if not ok or type(status) ~= "table" then return {} end
  return {
    memory_current = pick(status, { "memory_current", "MemoryCurrent" }),
    tasks_current = pick(status, { "tasks_current", "TasksCurrent" }),
    cpu_usage_nsec = pick(status, { "cpu_usage_nsec", "CPUUsageNSec" }),
    n_restarts = pick(status, { "n_restarts", "NRestarts" }),
    unit_file_state = pick(status, { "unit_file_state", "UnitFileState" }),
    fragment_path = pick(status, { "fragment_path", "FragmentPath" }),
    drop_in_paths = pick(status, { "drop_in_paths", "DropInPaths" }),
    main_pid = pick(status, { "main_pid", "MainPID" }),
    exec_main_pid = pick(status, { "exec_main_pid", "ExecMainPID" }),
    exec_main_status = pick(status, { "exec_main_status", "ExecMainStatus" }),
    exec_start = pick(status, { "exec_start", "ExecStart" }),
    restart = pick(status, { "restart", "Restart" }),
    restart_usec = pick(status, { "restart_usec", "RestartUSec" }),
    user = pick(status, { "user", "User" }),
    group = pick(status, { "group", "Group" }),
    working_directory = pick(status, { "working_directory", "WorkingDirectory" }),
    memory_accounting = pick(status, { "memory_accounting", "MemoryAccounting" }),
    cpu_accounting = pick(status, { "cpu_accounting", "CPUAccounting" }),
    tasks_accounting = pick(status, { "tasks_accounting", "TasksAccounting" }),
  }
end

local function stats_from_systemctl_show(unit)
  if type(shell) ~= "table" or type(shell.exec) ~= "function" then return {} end

  local cmd = "systemctl show"
  for _, property in ipairs(SHOW_PROPERTIES) do
    cmd = cmd .. " --property=" .. property
  end
  cmd = cmd .. " -- " .. shell_quote(unit)
  local ok, result = pcall(shell.exec, cmd)
  if not ok or type(result) ~= "table" or result.status ~= 0 then return {} end

  local out = {}
  for line in tostring(result.stdout or ""):gmatch("[^\n]+") do
    local key, value = line:match("^([^=]+)=(.*)$")
    if key then out[key] = value end
  end
  return {
    memory_current = out.MemoryCurrent,
    tasks_current = out.TasksCurrent,
    cpu_usage_nsec = out.CPUUsageNSec,
    n_restarts = out.NRestarts,
    unit_file_state = out.UnitFileState,
    fragment_path = out.FragmentPath,
    drop_in_paths = out.DropInPaths,
    main_pid = out.MainPID,
    exec_main_pid = out.ExecMainPID,
    exec_main_status = out.ExecMainStatus,
    exec_start = out.ExecStart,
    restart = out.Restart,
    restart_usec = out.RestartUSec,
    user = out.User,
    group = out.Group,
    working_directory = out.WorkingDirectory,
    memory_accounting = out.MemoryAccounting,
    cpu_accounting = out.CPUAccounting,
    tasks_accounting = out.TasksAccounting,
  }
end

local function merged_stats(unit)
  local primary = stats_from_unit_status(unit)
  if primary.memory_current and primary.tasks_current and primary.cpu_usage_nsec then
    return primary
  end

  local fallback = stats_from_systemctl_show(unit)
  local out = {}
  for _, field in ipairs(INFO_FIELDS) do
    out[field] = primary[field] or fallback[field]
  end
  return out
end

local function decorate(row)
  local unit = row.name or row.unit or ""
  local is_service = M.valid_service_name(unit)
  if not is_service then
    row.memory = "—"
    row.tasks = nil
    row.tasks_label = "—"
    row.cpu_time = "—"
    row.restarts = nil
    row.restarts_label = "—"
    row.restart_allowed = false
    row.action_allowed = false
    row.has_details = false
    row.details = {}
    return row
  end

  local stats = merged_stats(unit)
  local tasks = number_value(stats.tasks_current)
  local restarts = number_value(stats.n_restarts)
  row.memory = M.fmt_bytes(stats.memory_current)
  row.tasks = tasks
  row.tasks_label = tasks and tostring(math.floor(tasks)) or "—"
  row.cpu_time = M.fmt_cpu_time(stats.cpu_usage_nsec)
  row.restarts = restarts
  row.restarts_label = restarts and tostring(math.floor(restarts)) or "—"
  row.restart_allowed = true
  row.action_allowed = true
  row.dom_id = dom_id(unit)
  row.details = detail_rows(row, stats)
  row.has_details = #row.details > 0
  return row
end

function M.enrich(units)
  local out = {}
  for _, u in ipairs(units or {}) do
    local row = {}
    for k, v in pairs(u) do row[k] = v end
    out[#out + 1] = decorate(row)
  end
  return out
end

function M.action(unit, action)
  action = tostring(action or "")
  if not ALLOWED_ACTIONS[action] then
    return { ok = false, error = "unsupported service action: " .. action }
  end
  if not M.valid_service_name(unit) then
    return { ok = false, error = "unit must be a valid .service name" }
  end
  if type(systemd) ~= "table" then
    return { ok = false, error = "systemd builtin is unavailable" }
  end

  if type(systemd.unit_action) == "function" then
    local ok, result = pcall(systemd.unit_action, unit, action, { timeout = 60 })
    if not ok then return { ok = false, error = tostring(result) } end
    if type(result) == "table" and result.status ~= 0 then
      local msg = trim(result.stderr ~= "" and result.stderr or result.stdout)
      return { ok = false, error = msg ~= "" and msg or ("systemctl " .. action .. " failed") }
    end
    return { ok = true }
  end

  if type(systemd[action]) == "function" then
    local ok, result = pcall(systemd[action], unit)
    if not ok then return { ok = false, error = tostring(result) } end
    return { ok = true, job = result }
  end

  return { ok = false, error = "systemd " .. action .. " action is unavailable" }
end

function M.restart(unit)
  return M.action(unit, "restart")
end

function M.start(unit)
  return M.action(unit, "start")
end

function M.stop(unit)
  return M.action(unit, "stop")
end

return M
