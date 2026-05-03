local render = require("pages.render")
local ctx = require("hostops.ctx")
local M = {}

local function pcall_or_empty(fn, ...)
  local ok, result = pcall(fn, ...)
  if ok and type(result) == "table" then return result end
  return {}
end

local function fmt_ts(us)
  if not us or us == 0 then return "" end
  local s = math.floor(us / 1000000)
  return os.date("%H:%M:%S", s)
end

function M.page(req)
  local snap    = ctx.state.snapshot()
  local initial = pcall_or_empty(systemd.journal, { lines = 50 })

  -- Annotate each entry with a human-readable ts_pretty
  for _, e in ipairs(initial) do
    e.ts_pretty = fmt_ts(e.ts)
  end

  return render.render("logs", {
    nav_active = "logs",
    host       = snap.host,
    machines   = snap.machines,
    initial    = initial,
  }, req)
end

return M
