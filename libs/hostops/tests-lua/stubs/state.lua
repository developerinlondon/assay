--! Stub `state` service for hostops smoke tests.
--!
--! Returns a deterministic snapshot with two fixture machines so the
--! dashboard sidebar and machines list have something to render.

local M = {}

local FIXTURE_MACHINES = {
  {
    name      = "agentx",
    status    = "running",
    ip        = "10.10.0.10",
    image     = "debian-bookworm",
    started   = "2026-04-30T12:00:00Z",
    cpu_pct   = 3.4,
    mem_used  = 128 * 1024 * 1024,
    mem_total = 1024 * 1024 * 1024,
  },
  {
    name      = "k3s-server",
    status    = "running",
    ip        = "10.10.0.11",
    image     = "debian-bookworm",
    started   = "2026-04-29T08:30:00Z",
    cpu_pct   = 12.1,
    mem_used  = 512 * 1024 * 1024,
    mem_total = 2048 * 1024 * 1024,
  },
}

-- All numeric fields are typed as numbers so the template's
-- `| round | int` filters work without coercion errors.
local FIXTURE_HOST = {
  name         = "test-host",
  ip           = "10.10.0.1",
  kernel       = "6.14.0-test",
  uptime_secs  = 86400,
  cpu_pct      = 12.5,
  num_procs    = 234,
  load_one     = 0.5,
  load_five    = 0.4,
  load_fifteen = 0.3,
  mem_total    = 16 * 1024 * 1024 * 1024,
  mem_used     =  4 * 1024 * 1024 * 1024,
  mem_avail    = 12 * 1024 * 1024 * 1024,
  disk_total   = 500 * 1024 * 1024 * 1024,
  disk_used    = 200 * 1024 * 1024 * 1024,
  disk_free    = 300 * 1024 * 1024 * 1024,
  disk_pct     = 40.0,
}

function M.start() end
function M.bump() end

function M.snapshot()
  return {
    host     = FIXTURE_HOST,
    machines = FIXTURE_MACHINES,
  }
end

function M.machine_deep(name)
  for _, m in ipairs(FIXTURE_MACHINES) do
    if m.name == name then
      return {
        info     = m,
        services = {},
        cron     = {},
        journal  = {},
      }
    end
  end
  return nil
end

return M
