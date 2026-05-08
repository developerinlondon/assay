--! sysops.vault.sealing - vault seal/unseal/init operations.

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
  local sealing = {}

  function sealing.status()
    local resp = engine.get(BASE .. "/sys/seal-status")
    return decode(resp)
  end

  function sealing.seal()
    local resp = engine.post(BASE .. "/sys/seal", {})
    return decode(resp)
  end

  function sealing.unseal(share_b64)
    local resp = engine.post(BASE .. "/sys/unseal", { key = share_b64 })
    return decode(resp)
  end

  function sealing.init(shares_count, threshold)
    local resp = engine.post(BASE .. "/sys/init", {
      secret_shares = shares_count,
      secret_threshold = threshold,
    })
    return decode(resp)
  end

  return sealing
end

return M
