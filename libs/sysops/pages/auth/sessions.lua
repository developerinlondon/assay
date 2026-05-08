local render = require("pages.render")
local ctx    = require("sysops.ctx")
local auth   = require("sysops.auth")

local M = {}

local function urlenc(s)
  return (tostring(s or "")):gsub("([^%w%-_%.~])", function(c)
    return string.format("%%%02X", string.byte(c))
  end)
end

local function id_short(id)
  if not id then return "?" end
  local s = tostring(id)
  if #s > 8 then return s:sub(1, 8) .. "…" end
  return s
end

function M.page(req)
  local q       = (req and req.params) or {}
  local user_id = q.user_id or ""
  local sdk     = auth.new(ctx.engine).sessions
  local opts    = {}
  if user_id ~= "" then opts.user_id = user_id end
  local data, err = sdk.list(opts)
  local sessions = {}
  if data and type(data.sessions) == "table" then
    for _, s in ipairs(data.sessions) do
      sessions[#sessions + 1] = {
        id         = s.id,
        id_short   = id_short(s.id),
        user_id    = s.user_id,
        user_short = id_short(s.user_id),
        created_at = s.created_at,
        expires_at = s.expires_at,
        ip_hash    = s.ip_hash,
        ua_full    = s.user_agent or "",
        ua_short   = s.user_agent and s.user_agent:sub(1, 40) or "—",
      }
    end
  end
  return render.render("auth/sessions", {
    nav_active  = "auth:sessions",
    title       = "Sessions · auth",
    page_title  = "Sessions",
    sessions    = sessions,
    total       = (data and data.total) or #sessions,
    user_id     = user_id,
    error       = err,
    status      = err and err.status or 200,
    error_msg   = q.error and q.error or nil,
    ok_msg      = q.ok    and q.ok    or nil,
  }, req)
end

function M.revoke(req)
  local path = (req and req.path) or ""
  local id   = path:match("^/auth/sessions/([^/]+)/revoke$")
  if not id then return { status = 404, body = "not found" } end
  local sdk = auth.new(ctx.engine).sessions
  local _, err = sdk.revoke(id)
  if err then
    return { status = 303, headers = { Location = "/auth/sessions?error=" .. urlenc(tostring(err.status) .. ":revoke failed") } }
  end
  return { status = 303, headers = { Location = "/auth/sessions" } }
end

return M
