--! sysops.vault.transit - transit encryption engine operations.

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
  local transit = {}

  function transit.keys()
    local resp = engine.get(BASE .. "/transit/keys")
    return decode(resp)
  end

  function transit.create(name, algo)
    local resp = engine.post(BASE .. "/transit/keys/" .. encode.segment(name), { type = algo })
    return decode(resp)
  end

  function transit.rotate(name)
    local resp = engine.post(BASE .. "/transit/keys/" .. encode.segment(name) .. "/rotate", {})
    return decode(resp)
  end

  function transit.encrypt(name, plaintext)
    local resp = engine.post(
      BASE .. "/transit/encrypt/" .. encode.segment(name),
      { plaintext = plaintext }
    )
    return decode(resp)
  end

  function transit.decrypt(name, ciphertext)
    local resp = engine.post(
      BASE .. "/transit/decrypt/" .. encode.segment(name),
      { ciphertext = ciphertext }
    )
    return decode(resp)
  end

  return transit
end

return M
