--! sysops.vault.collections - bitwarden-compat collection/folder management.

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
  local collections = {}

  function collections.list()
    local resp = engine.get(BASE .. "/folders")
    return decode(resp)
  end

  function collections.create(name, description)
    local resp = engine.post(BASE .. "/folders", { name = name, description = description })
    return decode(resp)
  end

  return collections
end

return M
