--! sysops.vault.kv - KV v2 operations via the engine HTTP API.

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
  local kv = {}

  function kv.get(path)
    local resp = engine.get(BASE .. "/kv/" .. encode.path(path))
    return decode(resp)
  end

  function kv.put(path, value)
    local resp = engine.put(BASE .. "/kv/" .. encode.path(path), { data = value })
    return decode(resp)
  end

  function kv.delete(path, version)
    local url = BASE .. "/kv/" .. encode.path(path)
    if version then url = url .. "?version=" .. tostring(version) end
    local resp = engine.delete(url)
    return decode(resp)
  end

  function kv.list(prefix)
    local url = BASE .. "/kv-list"
    if type(prefix) == "string" and prefix ~= "" then
      url = url .. "/" .. encode.path(prefix)
    end
    local resp = engine.get(url)
    return decode(resp)
  end

  function kv.meta(path)
    local resp = engine.get(BASE .. "/kv-meta/" .. encode.path(path))
    return decode(resp)
  end

  function kv.destroy(path, versions)
    local resp = engine.post(BASE .. "/kv-destroy/" .. encode.path(path), { versions = versions })
    return decode(resp)
  end

  function kv.undelete(path, versions)
    local resp = engine.post(BASE .. "/kv-undelete/" .. encode.path(path), { versions = versions })
    return decode(resp)
  end

  return kv
end

return M
