--! sysops.vault.share - one-time secret share mint/redeem/revoke.

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
  local share = {}

  function share.mint(opts)
    local resp = engine.post(BASE .. "/share", opts or {})
    return decode(resp)
  end

  function share.redeem(token)
    local resp = engine.get(BASE .. "/share/" .. encode.segment(token))
    return decode(resp)
  end

  function share.revoke(token)
    local resp = engine.post(BASE .. "/share/revoke", { token = token })
    return decode(resp)
  end

  return share
end

return M
