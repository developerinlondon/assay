--! sysops.vault.dynamic - dynamic credential lease operations.

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
  local dynamic = {}

  function dynamic.lease(provider, role)
    local resp = engine.post(
      BASE .. "/dynamic/" .. encode.segment(provider) .. "/" .. encode.segment(role) .. "/lease",
      {}
    )
    return decode(resp)
  end

  function dynamic.list(provider)
    local url = BASE .. "/dynamic/leases"
    if type(provider) == "string" and provider ~= "" then
      url = url .. "?provider=" .. encode.segment(provider)
    end
    local resp = engine.get(url)
    return decode(resp)
  end

  function dynamic.revoke(id)
    local resp = engine.delete(BASE .. "/dynamic/leases/" .. encode.segment(id))
    return decode(resp)
  end

  return dynamic
end

return M
