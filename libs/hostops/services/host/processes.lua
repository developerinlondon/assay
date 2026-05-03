local M = {}

-- Per-pid CPU tick cache: { [pid] = { ticks, time } }
local _prev = {}

local CLK_TCK = 100  -- sysconf(_SC_CLK_TCK) fallback

local function rss_to_bytes(rss_pages)
  return (rss_pages or 0) * 4096
end

local function fmt_cmdline(raw)
  if not raw or raw == "" then return "" end
  local s = raw:gsub("%z", " "):gsub("%s+$", "")
  if #s > 80 then s = s:sub(1, 77) .. "..." end
  return s
end

-- Collect all pids recursively from cgroup tree.
local function collect_pids(path, out)
  local ok, pids = pcall(cgroup.procs, path)
  if ok and pids then
    for _, pid in ipairs(pids) do
      out[#out + 1] = pid
    end
  end
  local ok2, children = pcall(cgroup.list, path)
  if ok2 and children then
    for _, child in ipairs(children) do
      collect_pids(path .. "/" .. child, out)
    end
  end
end

function M.top_in_cgroup(cgroup_path, opts)
  opts = opts or {}
  local top_n = opts.top or 10
  local now = time()

  local pids = {}
  collect_pids(cgroup_path, pids)

  local procs = {}
  for _, pid in ipairs(pids) do
    local ok_stat, stat = pcall(linux.proc_stat, pid)
    if not ok_stat then goto next_pid end

    local utime  = stat.utime  or 0
    local stime  = stat.stime  or 0
    local ticks  = utime + stime
    local cpu_pct = 0.0

    local prev = _prev[pid]
    if prev then
      local delta_ticks = ticks - prev.ticks
      local delta_secs  = now - prev.time
      if delta_secs > 0 then
        cpu_pct = (delta_ticks / CLK_TCK / delta_secs) * 100.0
        if cpu_pct < 0 then cpu_pct = 0.0 end
      end
    end
    _prev[pid] = { ticks = ticks, time = now }

    local rss_bytes = 0
    local state = stat.state or "?"
    local comm  = stat.comm  or ""
    local threads = stat.num_threads or 1

    local ok_status, status = pcall(linux.proc_status, pid)
    if ok_status and status then
      rss_bytes = status.vm_rss or 0
      if comm == "" then comm = status.name or "" end
    end

    local cmdline = ""
    local ok_cmd, raw_cmd = pcall(fs.read, "/proc/" .. pid .. "/cmdline")
    if ok_cmd and raw_cmd then
      cmdline = fmt_cmdline(raw_cmd)
    end
    if cmdline == "" then cmdline = comm end

    procs[#procs + 1] = {
      pid       = pid,
      comm      = comm,
      state     = state,
      cpu_pct   = cpu_pct,
      rss_bytes = rss_bytes,
      threads   = threads,
      cmdline   = cmdline,
    }

    ::next_pid::
  end

  -- Sort descending by cpu_pct
  table.sort(procs, function(a, b) return a.cpu_pct > b.cpu_pct end)

  -- Cap at top_n
  local result = {}
  for i = 1, math.min(top_n, #procs) do
    result[i] = procs[i]
  end
  return result
end

return M
