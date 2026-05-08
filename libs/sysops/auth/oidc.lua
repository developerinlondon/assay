--! sysops.auth.oidc - OIDC clients, upstreams, and JWKS SDK for assay-engine auth admin.
--!
--! Wraps: GET    /api/v1/engine/auth/admin/oidc/clients
--!        POST   /api/v1/engine/auth/admin/oidc/clients
--!        PUT    /api/v1/engine/auth/admin/oidc/clients/{id}
--!        DELETE /api/v1/engine/auth/admin/oidc/clients/{id}
--!        POST   /api/v1/engine/auth/admin/oidc/clients/{id}/rotate-secret
--!        GET    /api/v1/engine/auth/admin/oidc/upstream
--!        POST   /api/v1/engine/auth/admin/oidc/upstream
--!        DELETE /api/v1/engine/auth/admin/oidc/upstream/{slug}
--!        GET    /api/v1/engine/auth/admin/jwks
--!
--! Note: clients/ and upstream/ return BARE JSON arrays, not {items: [...]}.
--! JWKS returns {keys: [...]}.

local M = {}

local encode = require("sysops.vault.encode")

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
    local resp = engine.get("/api/v1/engine/auth/admin/oidc/clients")
    return result(resp)
  end

  function self.create_client(fields)
    local resp = engine.post("/api/v1/engine/auth/admin/oidc/clients", fields)
    return result(resp)
  end

  function self.update_client(id, fields)
    local resp = engine.put("/api/v1/engine/auth/admin/oidc/clients/" .. encode.segment(id), fields)
    return result(resp)
  end

  function self.delete_client(id)
    local resp = engine.delete("/api/v1/engine/auth/admin/oidc/clients/" .. encode.segment(id))
    return result(resp)
  end

  function self.rotate_client_secret(id)
    local resp = engine.post("/api/v1/engine/auth/admin/oidc/clients/" .. encode.segment(id) .. "/rotate-secret", {})
    return result(resp)
  end

  function self.upstreams()
    local resp = engine.get("/api/v1/engine/auth/admin/oidc/upstream")
    return result(resp)
  end

  function self.upsert_upstream(fields)
    local resp = engine.post("/api/v1/engine/auth/admin/oidc/upstream", fields)
    return result(resp)
  end

  function self.delete_upstream(slug)
    local resp = engine.delete("/api/v1/engine/auth/admin/oidc/upstream/" .. encode.segment(slug))
    return result(resp)
  end

  function self.jwks()
    local resp = engine.get("/api/v1/engine/auth/admin/jwks")
    return result(resp)
  end

  return self
end

return M
