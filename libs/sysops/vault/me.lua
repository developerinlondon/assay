--! sysops.vault.me - personal vault (bitwarden-compat) operations.
--!
--! Note: sync payloads can be large for users with many items. Engine does
--! not yet support pagination on these endpoints; 0.1.5 issues one-shot calls.

local encode = require("sysops.vault.encode")

local BASE = "/api/v1/vault"

local function decode(resp)
  if resp.status == 0 then
    return nil, { status = 0, body = "engine unreachable" }
  end
  if resp.status < 200 or resp.status >= 300 then
    return nil, { status = resp.status, body = resp.body }
  end
  local body = resp.body
  if type(body) == "string" and body ~= "" then
    local ok, decoded = pcall(json.parse, body)
    if ok and type(decoded) == "table" then body = decoded end
  end
  return body or {}, nil
end

local M = {}

function M.new(engine)
  local me = {}

  function me.sync(user_id)
    local resp = engine.get(BASE .. "/me/" .. encode.segment(user_id))
    return decode(resp)
  end

  function me.ciphers(user_id)
    local resp = engine.get(BASE .. "/me/" .. encode.segment(user_id) .. "/items")
    return decode(resp)
  end

  function me.folders(user_id)
    local resp = engine.get(BASE .. "/me/" .. encode.segment(user_id) .. "/folders")
    return decode(resp)
  end

  function me.profile(user_id)
    local resp = engine.get(BASE .. "/me/" .. encode.segment(user_id) .. "/profile")
    return decode(resp)
  end

  return me
end

return M
