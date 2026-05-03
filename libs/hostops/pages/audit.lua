local render = require("pages.render")
local ctx = require("hostops.ctx")
local M = {}

local function ts_pretty(ts)
  if not ts or ts == 0 then return "—" end
  local now  = os.time()
  local diff = now - ts
  if diff < 60 then
    return tostring(diff) .. "s ago"
  elseif diff < 3600 then
    return tostring(math.floor(diff / 60)) .. "m ago"
  elseif diff < 86400 then
    return tostring(math.floor(diff / 3600)) .. "h ago"
  else
    local t = os.date("*t", ts)
    return string.format("%02d:%02d:%02d", t.hour, t.min, t.sec)
  end
end

-- Extract action prefix: "machine.restart" -> "machine"
local function action_prefix(action)
  if not action then return "other" end
  return action:match("^([^%.]+)") or "other"
end

function M.audit(req)
  local snap   = ctx.state.snapshot()
  local q      = (req and req.params) or {}
  local search  = ((q.search or ""):lower())
  local action_filter = q.action or "all"

  local raw    = ctx.audit.recent(200)
  local entries = {}
  for _, e in ipairs(raw) do
    local action  = e.action or "?"
    local actor   = e.actor  or "?"
    local target  = e.target or "?"
    local prefix  = action_prefix(action)

    -- Action prefix filter
    if action_filter ~= "all" and prefix ~= action_filter then
      goto continue
    end

    -- Search filter across actor/action/target
    if search ~= "" then
      local haystack = (actor .. " " .. action .. " " .. target):lower()
      if not haystack:find(search, 1, true) then
        goto continue
      end
    end

    entries[#entries + 1] = {
      ts_pretty    = ts_pretty(e.ts),
      actor        = actor,
      action       = action,
      action_prefix = prefix,
      target       = target,
      result       = e.result or "?",
      ip           = e.ip     or "?",
    }

    ::continue::
  end

  return render.render("audit", {
    nav_active    = "audit",
    host          = snap.host,
    machines      = snap.machines,
    entries       = entries,
    search        = search,
    action_filter = action_filter,
  }, req)
end

return M
