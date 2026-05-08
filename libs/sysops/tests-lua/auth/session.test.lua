--! sysops.auth.session tests
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;;' \
--!     assay libs/sysops/tests-lua/auth/session.test.lua

local session_mod = require("sysops.auth.session")

local function fake_engine(responses)
  local calls = {}
  local function do_call(method, path, body)
    table.insert(calls, { method = method, path = path, body = body })
    local key = method .. " " .. path
    if responses[key] then return responses[key] end
    return { status = 200, body = { ok = true } }
  end
  local e = {
    calls  = calls,
    get    = function(p)    return do_call("GET",    p)       end,
    post   = function(p, b) return do_call("POST",   p, b)   end,
    put    = function(p, b) return do_call("PUT",    p, b)   end,
    delete = function(p)    return do_call("DELETE", p)       end,
  }
  return e
end

print("[sysops.auth.session]")

-- login success
do
  local e = fake_engine({})
  local sdk = session_mod.new(e)
  local data, err = sdk.login("alice@example.com", "s3cret")
  assert.not_nil(data, "login returns data on 200")
  assert.eq(err, nil, "login no error on 200")
  assert.eq(e.calls[1].method, "POST", "login uses POST")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/login", "login path")
  assert.eq(e.calls[1].body.email, "alice@example.com", "login email")
  assert.eq(e.calls[1].body.password, "s3cret", "login password")
  print("  ok login success")
end

-- login 401
do
  local e = fake_engine({
    ["POST /api/v1/engine/auth/login"] = { status = 401, body = { error = "bad creds" } },
  })
  local sdk = session_mod.new(e)
  local data, err = sdk.login("x@y.com", "wrong")
  assert.eq(data, nil, "login nil on 401")
  assert.not_nil(err, "login error on 401")
  assert.eq(err.status, 401, "login err.status")
  print("  ok login 401")
end

-- login 503
do
  local e = fake_engine({
    ["POST /api/v1/engine/auth/login"] = { status = 503, body = nil },
  })
  local sdk = session_mod.new(e)
  local data, err = sdk.login("x@y.com", "pw")
  assert.eq(data, nil, "login nil on 503")
  assert.eq(err.status, 503, "login err.status 503")
  print("  ok login 503")
end

-- login network failure (status=0)
do
  local e = fake_engine({
    ["POST /api/v1/engine/auth/login"] = { status = 0, body = nil },
  })
  local sdk = session_mod.new(e)
  local data, err = sdk.login("x@y.com", "pw")
  assert.eq(data, nil, "login nil on network failure")
  assert.eq(err.status, 0, "login err.status 0")
  print("  ok login network failure")
end

-- logout success
do
  local e = fake_engine({})
  local sdk = session_mod.new(e)
  local data, err = sdk.logout()
  assert.not_nil(data, "logout data on 200")
  assert.eq(err, nil, "logout no error")
  assert.eq(e.calls[1].method, "DELETE", "logout uses DELETE")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/session", "logout path")
  print("  ok logout success")
end

-- whoami success
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/whoami"] = { status = 200, body = { id = "u1", email = "a@b.com" } },
  })
  local sdk = session_mod.new(e)
  local data, err = sdk.whoami()
  assert.not_nil(data, "whoami data on 200")
  assert.eq(err, nil, "whoami no error")
  assert.eq(data.id, "u1", "whoami id")
  assert.eq(e.calls[1].method, "GET", "whoami uses GET")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/whoami", "whoami path")
  print("  ok whoami success")
end

-- whoami 404
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/whoami"] = { status = 404, body = nil },
  })
  local sdk = session_mod.new(e)
  local data, err = sdk.whoami()
  assert.eq(data, nil, "whoami nil on 404")
  assert.eq(err.status, 404, "whoami err 404")
  print("  ok whoami 404")
end

-- passkey register start
do
  local e = fake_engine({})
  local sdk = session_mod.new(e)
  local data, err = sdk.passkey.register_start({ user_id = "u1" })
  assert.not_nil(data, "passkey.register_start data")
  assert.eq(err, nil, "passkey.register_start no error")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/passkey/register/start", "register_start path")
  print("  ok passkey.register_start")
end

-- passkey register finish
do
  local e = fake_engine({})
  local sdk = session_mod.new(e)
  local data, err = sdk.passkey.register_finish({ credential = "cred" })
  assert.not_nil(data, "passkey.register_finish data")
  assert.eq(err, nil, "passkey.register_finish no error")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/passkey/register/finish", "register_finish path")
  print("  ok passkey.register_finish")
end

-- passkey auth start
do
  local e = fake_engine({})
  local sdk = session_mod.new(e)
  local data, err = sdk.passkey.auth_start({})
  assert.not_nil(data, "passkey.auth_start data")
  assert.eq(err, nil, "passkey.auth_start no error")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/passkey/auth/start", "auth_start path")
  print("  ok passkey.auth_start")
end

-- passkey auth finish
do
  local e = fake_engine({})
  local sdk = session_mod.new(e)
  local data, err = sdk.passkey.auth_finish({ assertion = "a" })
  assert.not_nil(data, "passkey.auth_finish data")
  assert.eq(err, nil, "passkey.auth_finish no error")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/passkey/auth/finish", "auth_finish path")
  print("  ok passkey.auth_finish")
end

print("[sysops.auth.session] all passed")
