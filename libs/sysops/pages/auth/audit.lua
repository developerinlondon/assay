local render = require("pages.render")
local ctx    = require("sysops.ctx")
local auth   = require("sysops.auth")

local M = {}

function M.page(req)
  local q        = (req and req.params) or {}
  local actor_q  = q.actor  or ""
  local action_q = q.action or ""
  local since_q  = q.since  or ""
  local sdk      = auth.new(ctx.engine).audit
  local opts     = {}
  if since_q ~= "" then opts.since = since_q end
  local data, err = sdk.list(opts)
  local raw     = (data and type(data.entries) == "table") and data.entries or {}
  local entries = {}
  for _, e in ipairs(raw) do
    if actor_q ~= "" and not tostring(e.actor or ""):lower():find(actor_q:lower(), 1, true) then
      goto continue
    end
    if action_q ~= "" and not tostring(e.action or ""):lower():find(action_q:lower(), 1, true) then
      goto continue
    end
    local details = e.details
    local collapsed = false
    if type(details) == "table" then
      details = (function()
        local ok, s = pcall(require("json").encode, details)
        return ok and s or tostring(details)
      end)()
    end
    details = tostring(details or "")
    collapsed = #details > 120
    entries[#entries + 1] = {
      ts          = e.ts or e.created_at,
      actor       = e.actor,
      action      = e.action,
      target_type = e.target_type,
      target_id   = e.target_id,
      status      = e.status,
      details     = details,
      collapsed   = collapsed,
    }
    ::continue::
  end
  local enabled = not (data and data.enabled == false)
  return render.render("auth/audit", {
    nav_active  = "auth:audit",
    title       = "Audit · auth",
    page_title  = "Audit log",
    entries     = entries,
    total       = (data and data.total) or #entries,
    enabled     = enabled,
    actor_q     = actor_q,
    action_q    = action_q,
    since_q     = since_q,
    error       = err,
    status      = err and err.status or 200,
  }, req)
end

return M
