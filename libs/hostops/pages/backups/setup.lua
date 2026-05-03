-- pages/backups_setup.lua
--
-- POST handlers for the setup wizard:
--   /api/backups/setup/test     — non-mutating connection probe
--   /api/backups/setup/init     — runs rustic init, persists state
--   /api/backups/reconfigure    — wipes and re-inits

local render  = require("pages.render")
local form    = require("pages.form")
local backups = require("services.host.backups")

local M = {}

local function actor_from(req)
  local h = (req and req.headers) or {}
  return h["Cf-Access-Authenticated-User-Email"]
      or h["cf-access-authenticated-user-email"]
      or "local-dev"
end

local function build_args(req)
  local f = form.parse(req)
  -- Build the s3:URL from the wizard's separate fields. Keeps the TOML
  -- as a single readable URL string while operators see logical fields.
  local endpoint = (f.endpoint or ""):gsub("/+$", "")
  local bucket   = f.bucket or ""
  local prefix   = f.prefix or ""
  local url
  if endpoint ~= "" and bucket ~= "" then
    url = "s3:" .. endpoint .. "/" .. bucket
    if prefix ~= "" then url = url .. "/" .. prefix end
  end
  return {
    actor = actor_from(req),
    url = url,
    region = f.region,
    access_key_id = f.access_key_id,
    secret_access_key = f.secret_access_key,
    password = f.password,
    password_confirm = f.password_confirm,
    enable_virtual_host_style = f.enable_virtual_host_style == "on",
    schedule_hour = tonumber(f.schedule_hour),
    schedule_jitter = tonumber(f.schedule_jitter),
  }
end

function M.test(req)
  local a = build_args(req)
  if not a.url then
    return { status = 400, body = "endpoint + bucket required" }
  end
  local res = backups.test_connection({
    url = a.url,
    region = a.region,
    access_key_id = a.access_key_id,
    secret_access_key = a.secret_access_key,
    password = a.password,
  })
  return {
    status = 200,
    body = json.encode(res),
    headers = { ["Content-Type"] = "application/json" },
  }
end

function M.init(req)
  local a = build_args(req)
  if not a.url then
    return { status = 400, body = "endpoint + bucket required" }
  end
  local res = backups.init_repo(a)
  if res.ok then
    return { status = 303, headers = { ["Location"] = "/backups?init=ok" } }
  end
  return {
    status = 400,
    body = "init failed: " .. (res.error or "unknown"),
  }
end

function M.reconfigure(req)
  local a = build_args(req)
  if not a.url then
    return { status = 400, body = "endpoint + bucket required" }
  end
  local res = backups.reconfigure(a)
  if res.ok then
    return { status = 303, headers = { ["Location"] = "/backups?reconfig=ok" } }
  end
  return { status = 400, body = "reconfigure failed: " .. (res.error or "?") }
end

return M
