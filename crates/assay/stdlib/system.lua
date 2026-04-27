--- @module assay.system
--- @description Host introspection and control umbrella: re-exports linux/cgroup/systemd builtins + assay.cron, plus convenience aggregates (host_snapshot, machine_snapshot, machines).
--- @keywords system, host, linux, cgroup, systemd, cron, snapshot, machines, observability
--- @quickref M.linux | direct passthrough to the linux builtin
--- @quickref M.cgroup | direct passthrough to the cgroup builtin
--- @quickref M.systemd | direct passthrough to the systemd builtin
--- @quickref M.cron | direct passthrough to require("assay.cron")
--- @quickref M.host_snapshot() -> {cpu, mem, load, uptime, netdev, kernel} | One-call host vitals
--- @quickref M.machine_snapshot(name) -> {info, cgroup={cpu,memory,io,pids}, journal_tail} | Per-machine roll-up
--- @quickref M.machines() -> [{name, class, addresses, cgroup={cpu, memory, ...}}, ...] | list_machines + cgroup join

local cron = require("assay.cron")

local M = {
  linux   = type(linux)   == "table" and linux   or nil,
  cgroup  = type(cgroup)  == "table" and cgroup  or nil,
  systemd = type(systemd) == "table" and systemd or nil,
  cron    = cron,
}

local function require_linux()
  if not M.linux then error("assay.system: linux builtin not available (Linux only)") end
end

local function require_systemd()
  if not M.systemd then error("assay.system: systemd builtin not available (Linux only)") end
end

local function require_cgroup()
  if not M.cgroup then error("assay.system: cgroup builtin not available (Linux only)") end
end

local function pcall_or_nil(fn, ...)
  if not fn then return nil end
  local ok, val = pcall(fn, ...)
  if ok then return val end
  return nil
end

--- One-shot host vitals: CPU, memory, load, uptime, network, kernel info.
--- Every field is best-effort; missing builtins or read failures degrade
--- gracefully to nil so the caller can still render a partial dashboard.
--- @return {cpu, mem, load, uptime, netdev, kernel}
function M.host_snapshot()
  require_linux()
  return {
    cpu     = pcall_or_nil(M.linux.cpu_stat),
    mem     = pcall_or_nil(M.linux.meminfo),
    load    = pcall_or_nil(M.linux.loadavg),
    uptime  = pcall_or_nil(M.linux.uptime),
    netdev  = pcall_or_nil(M.linux.netdev),
    kernel  = pcall_or_nil(M.linux.kernel),
  }
end

--- Per-machine roll-up: machine info + cgroup utilization + last 20 journal
--- lines for the machine.
--- @param name string
--- @return {info, cgroup, journal_tail}
function M.machine_snapshot(name)
  require_systemd()
  require_cgroup()
  local cgroup_path = "/sys/fs/cgroup/machine.slice/systemd-nspawn@" .. name .. ".service"
  return {
    info = pcall_or_nil(M.systemd.machine_status, name),
    cgroup = {
      cpu    = pcall_or_nil(M.cgroup.cpu_stat, cgroup_path),
      memory = pcall_or_nil(M.cgroup.memory,   cgroup_path),
      io     = pcall_or_nil(M.cgroup.io,       cgroup_path),
      pids   = pcall_or_nil(M.cgroup.pids,     cgroup_path),
    },
    journal_tail = pcall_or_nil(M.systemd.journal, { machine = name, lines = 20 }),
  }
end

--- list_machines() with each entry enriched by its cgroup utilization
--- snapshot. Useful for an Overview-style dashboard that shows every nspawn
--- container at a glance.
--- @return [{name, class, addresses, cgroup={cpu, memory, pids}}, ...]
function M.machines()
  require_systemd()
  local items = pcall_or_nil(M.systemd.list_machines) or {}
  if not M.cgroup then return items end
  for _, m in ipairs(items) do
    local p = "/sys/fs/cgroup/machine.slice/systemd-nspawn@" .. (m.name or "") .. ".service"
    m.cgroup = {
      cpu    = pcall_or_nil(M.cgroup.cpu_stat, p),
      memory = pcall_or_nil(M.cgroup.memory,   p),
      pids   = pcall_or_nil(M.cgroup.pids,     p),
    }
  end
  return items
end

return M
