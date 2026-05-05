local sys = require("assay.system")
local M = {}

-- Previous cpu usage_usec snapshots: { [name] = { usage_usec, time } }
local _prev_cpu = {}

local function is_link_local_v6(addr)
  return type(addr) == "string" and addr:match("^fe80:")
end

local function first_routable_ip(addresses)
  if type(addresses) ~= "table" then return nil end
  for _, entry in ipairs(addresses) do
    local addr
    if type(entry) == "table" then
      addr = entry.ip or entry.address or entry[1]
    elseif type(entry) == "string" then
      addr = entry
    end
    if addr and not is_link_local_v6(addr) then
      -- strip prefix length if present: "10.10.0.5/24" -> "10.10.0.5"
      return addr:match("^([^/]+)")
    end
  end
  return nil
end

local function cgroup_path(name)
  return "/sys/fs/cgroup/machine.slice/systemd-nspawn@" .. name .. ".service"
end

local function state_class(state)
  if state == "running" then return "ok" end
  if state == "failed"  then return "err" end
  return "muted"
end

local function pill_class(state)
  if state == "running" then return "pill-ok" end
  if state == "failed"  then return "pill-err" end
  return "pill-muted"
end

function M.snapshot()
  local raw = sys.systemd.list_machines()
  local now = time()
  local result = {}

  for _, m in ipairs(raw) do
    if m.class ~= "container" then goto continue end

    local name = m.name
    local path = cgroup_path(name)

    -- CPU percent from usage_usec delta
    local cpu_pct = 0.0
    local ok_cpu, cpu_stat = pcall(cgroup.cpu_stat, path)
    if ok_cpu and cpu_stat then
      local prev = _prev_cpu[name]
      if prev then
        local delta_usec = (cpu_stat.usage_usec or 0) - prev.usage_usec
        local delta_wall  = (now - prev.time) * 1e6  -- convert secs to usec
        if delta_wall > 0 then
          cpu_pct = (delta_usec / delta_wall) * 100.0
          if cpu_pct < 0 then cpu_pct = 0.0 end
        end
      end
      _prev_cpu[name] = { usage_usec = cpu_stat.usage_usec or 0, time = now }
    end

    -- Memory
    local mem_used = 0
    local mem_max  = nil
    local ok_mem, mem_stat = pcall(cgroup.memory, path)
    if ok_mem and mem_stat then
      mem_used = mem_stat.current or 0
      mem_max  = mem_stat.max   -- may be nil (unlimited)
    end

    -- PIDs
    local procs = 0
    local ok_pids, pids_stat = pcall(cgroup.pids, path)
    if ok_pids and pids_stat then
      procs = pids_stat.current or 0
    end

    -- State: present in list_machines means running; no state field available
    local state = "running"

    -- IP address
    local ip = first_routable_ip(m.addresses)

    table.insert(result, {
      name        = name,
      state       = state,
      state_class = state_class(state),
      pill_class  = pill_class(state),
      ip          = ip or "",
      cpu_pct     = cpu_pct,
      mem_used    = mem_used,
      mem_max     = mem_max,
      procs       = procs,
      leader_pid  = m.leader_pid,
      root        = m.root_directory or "",
    })

    ::continue::
  end

  return result
end

return M
