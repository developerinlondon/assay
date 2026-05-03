local M = {}

local _prev_cpu = nil

local function fmt_kernel_short(ver)
  -- "Linux version 6.14.0-37-generic ..." -> "6.14.0-37-generic"
  return ver:match("Linux version (%S+)") or ver
end

function M.snapshot()
  local kern   = linux.kernel()
  local mem    = linux.meminfo()
  local load   = linux.loadavg()
  local up     = linux.uptime()
  local cpu2   = linux.cpu_stat()

  local cpu_pct = 0.0
  if _prev_cpu then
    local pct = linux.cpu_percent(_prev_cpu, cpu2)
    cpu_pct = pct.total_pct or 0.0
  end
  _prev_cpu = cpu2

  local disk_info = disk.usage("/")

  -- assay meminfo() returns kB values multiplied by 1024*1024 instead of 1024;
  -- divide by 1024 to get real bytes.
  local mem_scale = 1024
  return {
    cpu_pct       = cpu_pct,
    mem_used      = ((mem.total or 0) - (mem.available or 0)) / mem_scale,
    mem_total     = (mem.total or 0) / mem_scale,
    mem_avail     = (mem.available or 0) / mem_scale,
    load_one      = load.one or 0,
    load_five     = load.five or 0,
    load_fifteen  = load.fifteen or 0,
    uptime_secs   = up.uptime_secs or 0,
    hostname      = kern.hostname or "unknown",
    kernel        = fmt_kernel_short(kern.version or ""),
    num_procs     = load.total or 0,
    disk_used     = disk_info.used or 0,
    disk_total    = disk_info.total or 0,
    disk_pct      = disk_info.percent or 0,
    disk_free     = disk_info.free or 0,
  }
end

return M
