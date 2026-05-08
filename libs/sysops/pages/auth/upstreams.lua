local render = require("pages.render")
local ctx    = require("sysops.ctx")
local auth   = require("sysops.auth")
local form   = require("pages.form")

local M = {}

local function urlenc(s)
  return (tostring(s or "")):gsub("([^%w%-_%.~])", function(c)
    return string.format("%%%02X", string.byte(c))
  end)
end

local function nz(s)
  if s == nil or s == "" then return nil end
  return s
end

function M.page(req)
  local q   = (req and req.params) or {}
  local sdk = auth.new(ctx.engine).oidc
  local data, err = sdk.upstreams()
  -- Engine returns either a bare JSON array or {items:[...]} depending on
  -- version. Accept both shapes.
  local upstreams = {}
  if type(data) == "table" then
    if type(data.items) == "table" then
      upstreams = data.items
    elseif #data > 0 or next(data) == nil then
      upstreams = data
    end
  end
  return render.render("auth/upstreams", {
    nav_active  = "auth:upstreams",
    title       = "Upstreams · auth",
    page_title  = "Upstreams",
    upstreams   = upstreams,
    error       = err,
    status      = err and err.status or 200,
    error_msg   = q.error and q.error or nil,
    ok_msg      = q.ok    and q.ok    or nil,
  }, req)
end

function M.upsert(req)
  local f   = form.parse(req)
  local sdk = auth.new(ctx.engine).oidc
  if not nz(f.slug) then
    return { status = 303, headers = { Location = "/auth/upstreams?error=400:slug+required" } }
  end
  if not nz(f.issuer) then
    return { status = 303, headers = { Location = "/auth/upstreams?error=400:issuer+required" } }
  end
  local fields = {
    slug         = f.slug,
    issuer       = f.issuer,
    display_name = nz(f.display_name),
    client_id    = nz(f.client_id),
    client_secret = nz(f.client_secret),
    icon_url     = nz(f.icon_url),
    enabled      = (f.enabled == "true" or f.enabled == "on" or f.enabled == "1"),
  }
  local _, err = sdk.upsert_upstream(fields)
  if err then
    return {
      status  = 303,
      headers = {
        Location = "/auth/upstreams"
          .. "?error=" .. urlenc(("upsert failed (status %s)"):format(err.status or "?"))
          .. "&form_slug=" .. urlenc(f.slug or "")
          .. "&form_display_name=" .. urlenc(f.display_name or "")
          .. "&form_issuer=" .. urlenc(f.issuer or "")
          .. "&form_client_id=" .. urlenc(f.client_id or "")
          .. "&form_icon_url=" .. urlenc(f.icon_url or ""),
      },
    }
  end
  return { status = 303, headers = { Location = "/auth/upstreams?ok=provider+saved" } }
end

function M.delete(req)
  local path = (req and req.path) or ""
  local slug = path:match("^/auth/upstreams/([^/]+)/delete$")
  if not slug then return { status = 404, body = "not found" } end
  local sdk = auth.new(ctx.engine).oidc
  local _, err = sdk.delete_upstream(slug)
  if err then
    return { status = 303, headers = { Location = "/auth/upstreams?error=" .. urlenc(tostring(err.status) .. ":delete failed") } }
  end
  return { status = 303, headers = { Location = "/auth/upstreams?ok=provider+deleted" } }
end

return M
