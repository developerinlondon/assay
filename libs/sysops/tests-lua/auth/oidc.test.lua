--! sysops.auth.oidc tests
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;;' \
--!     assay libs/sysops/tests-lua/auth/oidc.test.lua

local oidc_mod = require("sysops.auth.oidc")

local function fake_engine(responses)
  local calls = {}
  local function do_call(method, path, body)
    table.insert(calls, { method = method, path = path, body = body })
    local key = method .. " " .. path
    if responses[key] then return responses[key] end
    return { status = 200, body = {} }
  end
  return {
    calls  = calls,
    get    = function(p)    return do_call("GET",    p)       end,
    post   = function(p, b) return do_call("POST",   p, b)   end,
    put    = function(p, b) return do_call("PUT",    p, b)   end,
    delete = function(p)    return do_call("DELETE", p)       end,
  }
end

print("[sysops.auth.oidc]")

-- clients success
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/oidc-clients"] = {
      status = 200,
      body   = { items = { { client_id = "app1" } } },
    },
  })
  local sdk = oidc_mod.new(e)
  local data, err = sdk.clients()
  assert.not_nil(data, "clients data on 200")
  assert.eq(err, nil, "clients no error")
  assert.eq(e.calls[1].method, "GET", "clients uses GET")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/oidc-clients", "clients path")
  print("  ok clients success")
end

-- clients 404
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/oidc-clients"] = { status = 404, body = nil },
  })
  local sdk = oidc_mod.new(e)
  local data, err = sdk.clients()
  assert.eq(data, nil, "clients nil on 404")
  assert.eq(err.status, 404, "clients err 404")
  print("  ok clients 404")
end

-- clients 503
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/oidc-clients"] = { status = 503, body = nil },
  })
  local sdk = oidc_mod.new(e)
  local data, err = sdk.clients()
  assert.eq(data, nil, "clients nil on 503")
  assert.eq(err.status, 503, "clients err 503")
  print("  ok clients 503")
end

-- clients network failure
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/oidc-clients"] = { status = 0, body = nil },
  })
  local sdk = oidc_mod.new(e)
  local data, err = sdk.clients()
  assert.eq(data, nil, "clients nil on network failure")
  assert.eq(err.status, 0, "clients err status 0")
  print("  ok clients network failure")
end

-- upstreams success
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/upstreams"] = {
      status = 200,
      body   = { items = { { id = "github" } } },
    },
  })
  local sdk = oidc_mod.new(e)
  local data, err = sdk.upstreams()
  assert.not_nil(data, "upstreams data on 200")
  assert.eq(err, nil, "upstreams no error")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/upstreams", "upstreams path")
  print("  ok upstreams success")
end

-- upstreams 503
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/upstreams"] = { status = 503, body = nil },
  })
  local sdk = oidc_mod.new(e)
  local data, err = sdk.upstreams()
  assert.eq(data, nil, "upstreams nil on 503")
  assert.eq(err.status, 503, "upstreams err 503")
  print("  ok upstreams 503")
end

-- jwks success
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/jwks"] = {
      status = 200,
      body   = { keys = { { kid = "k1", kty = "RSA" } } },
    },
  })
  local sdk = oidc_mod.new(e)
  local data, err = sdk.jwks()
  assert.not_nil(data, "jwks data on 200")
  assert.eq(err, nil, "jwks no error")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/jwks", "jwks path")
  print("  ok jwks success")
end

-- jwks 404
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/jwks"] = { status = 404, body = nil },
  })
  local sdk = oidc_mod.new(e)
  local data, err = sdk.jwks()
  assert.eq(data, nil, "jwks nil on 404")
  assert.eq(err.status, 404, "jwks err 404")
  print("  ok jwks 404")
end

print("[sysops.auth.oidc] all passed")
