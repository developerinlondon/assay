local render = require("pages.render")
local ctx    = require("sysops.ctx")
local vault  = require("sysops.vault")
local form   = require("pages.form")

local M = {}

local function nz(s)
  if s == nil then return nil end
  if type(s) == "string" and s == "" then return nil end
  return s
end

local function urlenc(s)
  return (tostring(s or "")):gsub("([^%w%-_%.~])", function(c)
    return string.format("%%%02X", string.byte(c))
  end)
end

function M.page(req)
  local q = (req and req.params) or {}
  return render.render("vault/share", {
    nav_active    = "vault:share",
    title         = "Share · Vault",
    page_title    = "Vault share",
    minted_token  = q.token      or "",
    minted_rev_ids = q.revocation_ids or "",
    minted_expires = q.expires_at or "",
    revoked_id    = q.revoked    or "",
    error_msg     = q.error      or "",
    status        = 200,
  }, req)
end

function M.mint(req)
  local f   = form.parse(req)
  local sdk = vault.new(ctx.engine).share
  if not nz(f.target_kind) then
    return { status = 303, headers = { Location = "/vault/share?error=400:target_kind+required" } }
  end
  if not nz(f.target_id) then
    return { status = 303, headers = { Location = "/vault/share?error=400:target_id+required" } }
  end
  local ttl = tonumber(f.ttl_secs)
  if not ttl then
    return { status = 303, headers = { Location = "/vault/share?error=400:ttl_secs+required" } }
  end
  local opts = { target_kind = f.target_kind, target_id = f.target_id, ttl_secs = ttl }
  if nz(f.max_uses)    then opts.max_uses    = tonumber(f.max_uses) end
  if nz(f.max_ip_cidr) then opts.max_ip_cidr = f.max_ip_cidr end
  local data, err = sdk.mint(opts)
  if err then
    return { status = 303, headers = { Location = "/vault/share?error=" .. urlenc(tostring(err.status) .. ":mint failed") } }
  end
  local b = type(data) == "table" and data or {}
  local rev_ids = type(b.revocation_ids) == "table"
    and table.concat(b.revocation_ids, ",") or ""
  return {
    status = 303,
    headers = {
      Location = "/vault/share?token=" .. urlenc(b.token or "")
        .. "&revocation_ids=" .. urlenc(rev_ids)
        .. "&expires_at=" .. urlenc(tostring(b.expires_at or "")),
    },
  }
end

function M.revoke(req)
  local f   = form.parse(req)
  local sdk = vault.new(ctx.engine).share
  if not nz(f.revocation_id) then
    return { status = 303, headers = { Location = "/vault/share?error=400:revocation_id+required" } }
  end
  local _, err = sdk.revoke(f.revocation_id)
  if err then
    return { status = 303, headers = { Location = "/vault/share?error=" .. urlenc(tostring(err.status) .. ":revoke failed") } }
  end
  return { status = 303, headers = { Location = "/vault/share?revoked=" .. urlenc(f.revocation_id) } }
end

return M
