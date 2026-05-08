--! sysops.auth.session - session management SDK for assay-engine auth.
--!
--! Wraps: POST /api/v1/engine/auth/login
--!        DELETE /api/v1/engine/auth/session
--!        GET    /api/v1/engine/auth/whoami
--!        POST   /api/v1/engine/auth/passkey/register/{start,finish}
--!        POST   /api/v1/engine/auth/passkey/auth/{start,finish}

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

  function self.login(email, password)
    local resp = engine.post("/api/v1/engine/auth/login", { email = email, password = password })
    return result(resp)
  end

  function self.logout()
    local resp = engine.delete("/api/v1/engine/auth/session")
    return result(resp)
  end

  function self.whoami()
    local resp = engine.get("/api/v1/engine/auth/whoami")
    return result(resp)
  end

  self.passkey = {}

  function self.passkey.register_start(opts)
    local resp = engine.post("/api/v1/engine/auth/passkey/register/start", opts or {})
    return result(resp)
  end

  function self.passkey.register_finish(opts)
    local resp = engine.post("/api/v1/engine/auth/passkey/register/finish", opts or {})
    return result(resp)
  end

  function self.passkey.auth_start(opts)
    local resp = engine.post("/api/v1/engine/auth/passkey/auth/start", opts or {})
    return result(resp)
  end

  function self.passkey.auth_finish(opts)
    local resp = engine.post("/api/v1/engine/auth/passkey/auth/finish", opts or {})
    return result(resp)
  end

  return self
end

return M
