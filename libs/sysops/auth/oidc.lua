--! sysops.auth.oidc - OIDC clients, upstreams, and JWKS SDK for assay-engine auth admin.
--!
--! Wraps: GET /api/v1/engine/auth/admin/oidc-clients
--!        GET /api/v1/engine/auth/admin/upstreams
--!        GET /api/v1/engine/auth/admin/jwks

local M = {}

local function ok2xx(status)
  return type(status) == "number" and status >= 200 and status < 300
end

local function result(resp)
  if not resp or not ok2xx(resp.status) then
    return nil, { status = (resp and resp.status) or 0, body = resp and resp.body }
  end
  return resp.body, nil
end

function M.new(engine)
  local self = {}

  function self.clients()
    local resp = engine.get("/api/v1/engine/auth/admin/oidc-clients")
    return result(resp)
  end

  function self.upstreams()
    local resp = engine.get("/api/v1/engine/auth/admin/upstreams")
    return result(resp)
  end

  function self.jwks()
    local resp = engine.get("/api/v1/engine/auth/admin/jwks")
    return result(resp)
  end

  return self
end

return M
