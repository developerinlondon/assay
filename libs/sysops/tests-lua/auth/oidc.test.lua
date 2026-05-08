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
    ["GET /api/v1/engine/auth/admin/oidc/clients"] = {
      status = 200,
      body   = { { client_id = "app1" } },
    },
  })
  local sdk = oidc_mod.new(e)
  local data, err = sdk.clients()
  assert.not_nil(data, "clients data on 200")
  assert.eq(err, nil, "clients no error")
  assert.eq(e.calls[1].method, "GET", "clients uses GET")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/oidc/clients", "clients path")
  print("  ok clients success")
end

-- clients 404
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/oidc/clients"] = { status = 404, body = nil },
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
    ["GET /api/v1/engine/auth/admin/oidc/clients"] = { status = 503, body = nil },
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
    ["GET /api/v1/engine/auth/admin/oidc/clients"] = { status = 0, body = nil },
  })
  local sdk = oidc_mod.new(e)
  local data, err = sdk.clients()
  assert.eq(data, nil, "clients nil on network failure")
  assert.eq(err.status, 0, "clients err status 0")
  print("  ok clients network failure")
end

-- create_client
do
  local e = fake_engine({
    ["POST /api/v1/engine/auth/admin/oidc/clients"] = {
      status = 200,
      body   = { client = { client_id = "new-app" }, client_secret = "s3cr3t" },
    },
  })
  local sdk = oidc_mod.new(e)
  local data, err = sdk.create_client({ name = "New App", redirect_uris = { "http://localhost/cb" } })
  assert.not_nil(data, "create_client data on 200")
  assert.eq(err, nil, "create_client no error")
  assert.eq(e.calls[1].method, "POST", "create_client uses POST")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/oidc/clients", "create_client path")
  print("  ok create_client success")
end

-- delete_client
do
  local e = fake_engine({
    ["DELETE /api/v1/engine/auth/admin/oidc/clients/my-app"] = { status = 204, body = nil },
  })
  -- 204 is not 2xx by strict < 300, it is — verify result helper accepts it
  local sdk = oidc_mod.new(e)
  local data, err = sdk.delete_client("my-app")
  assert.eq(err, nil, "delete_client no error on 204")
  assert.eq(e.calls[1].method, "DELETE", "delete_client uses DELETE")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/oidc/clients/my-app", "delete_client path")
  print("  ok delete_client success")
end

-- delete_client encodes segment
do
  local e = fake_engine({})
  local sdk = oidc_mod.new(e)
  sdk.delete_client("foo/bar baz")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/oidc/clients/foo%2Fbar%20baz", "delete_client encodes id")
  print("  ok delete_client encodes segment")
end

-- rotate_client_secret
do
  local e = fake_engine({
    ["POST /api/v1/engine/auth/admin/oidc/clients/my-app/rotate-secret"] = {
      status = 200,
      body   = { client_id = "my-app", client_secret = "newsecret" },
    },
  })
  local sdk = oidc_mod.new(e)
  local data, err = sdk.rotate_client_secret("my-app")
  assert.not_nil(data, "rotate data on 200")
  assert.eq(err, nil, "rotate no error")
  assert.eq(e.calls[1].method, "POST", "rotate uses POST")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/oidc/clients/my-app/rotate-secret", "rotate path")
  print("  ok rotate_client_secret success")
end

-- upstreams success
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/oidc/upstream"] = {
      status = 200,
      body   = { { slug = "github" } },
    },
  })
  local sdk = oidc_mod.new(e)
  local data, err = sdk.upstreams()
  assert.not_nil(data, "upstreams data on 200")
  assert.eq(err, nil, "upstreams no error")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/oidc/upstream", "upstreams path")
  print("  ok upstreams success")
end

-- upstreams 503
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/oidc/upstream"] = { status = 503, body = nil },
  })
  local sdk = oidc_mod.new(e)
  local data, err = sdk.upstreams()
  assert.eq(data, nil, "upstreams nil on 503")
  assert.eq(err.status, 503, "upstreams err 503")
  print("  ok upstreams 503")
end

-- upsert_upstream
do
  local e = fake_engine({
    ["POST /api/v1/engine/auth/admin/oidc/upstream"] = {
      status = 200,
      body   = { slug = "google", issuer = "https://accounts.google.com" },
    },
  })
  local sdk = oidc_mod.new(e)
  local data, err = sdk.upsert_upstream({ slug = "google", issuer = "https://accounts.google.com" })
  assert.not_nil(data, "upsert_upstream data on 200")
  assert.eq(err, nil, "upsert_upstream no error")
  assert.eq(e.calls[1].method, "POST", "upsert_upstream uses POST")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/oidc/upstream", "upsert_upstream path")
  print("  ok upsert_upstream success")
end

-- delete_upstream
do
  local e = fake_engine({
    ["DELETE /api/v1/engine/auth/admin/oidc/upstream/google"] = { status = 204, body = nil },
  })
  local sdk = oidc_mod.new(e)
  local data, err = sdk.delete_upstream("google")
  assert.eq(err, nil, "delete_upstream no error on 204")
  assert.eq(e.calls[1].method, "DELETE", "delete_upstream uses DELETE")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/oidc/upstream/google", "delete_upstream path")
  print("  ok delete_upstream success")
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
