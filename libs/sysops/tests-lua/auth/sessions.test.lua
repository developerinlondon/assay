--! sysops.auth.sessions tests
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;;' \
--!     assay libs/sysops/tests-lua/auth/sessions.test.lua

local sessions_mod = require("sysops.auth.sessions")

local function fake_engine(responses)
  local calls = {}
  local function do_call(method, path, body)
    table.insert(calls, { method = method, path = path, body = body })
    local key = method .. " " .. path
    if responses[key] then return responses[key] end
    return { status = 200, body = { items = {}, total = 0 } }
  end
  return {
    calls  = calls,
    get    = function(p)    return do_call("GET",    p)       end,
    post   = function(p, b) return do_call("POST",   p, b)   end,
    put    = function(p, b) return do_call("PUT",    p, b)   end,
    delete = function(p)    return do_call("DELETE", p)       end,
  }
end

print("[sysops.auth.sessions]")

-- list success (no filter)
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/sessions"] = {
      status = 200,
      body   = { items = { { id = "s1" } }, total = 1 },
    },
  })
  local sdk = sessions_mod.new(e)
  local data, err = sdk.list()
  assert.not_nil(data, "list data on 200")
  assert.eq(err, nil, "list no error")
  assert.eq(data.total, 1, "list total")
  assert.eq(e.calls[1].method, "GET", "list uses GET")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/sessions", "list path")
  print("  ok list no filter")
end

-- list with filters
do
  local e = fake_engine({})
  local sdk = sessions_mod.new(e)
  sdk.list({ user_id = "u1", active_only = "true", limit = 5 })
  local path = e.calls[1].path
  assert.eq(path:find("user_id=u1",      1, true) ~= nil, true, "list user_id param")
  assert.eq(path:find("active_only=true", 1, true) ~= nil, true, "list active_only param")
  assert.eq(path:find("limit=5",          1, true) ~= nil, true, "list limit param")
  print("  ok list with filters")
end

-- list 404
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/sessions"] = { status = 404, body = nil },
  })
  local sdk = sessions_mod.new(e)
  local data, err = sdk.list()
  assert.eq(data, nil, "list nil on 404")
  assert.eq(err.status, 404, "list err 404")
  print("  ok list 404")
end

-- list 503
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/sessions"] = { status = 503, body = nil },
  })
  local sdk = sessions_mod.new(e)
  local data, err = sdk.list()
  assert.eq(data, nil, "list nil on 503")
  assert.eq(err.status, 503, "list err 503")
  print("  ok list 503")
end

-- list network failure
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/sessions"] = { status = 0, body = nil },
  })
  local sdk = sessions_mod.new(e)
  local data, err = sdk.list()
  assert.eq(data, nil, "list nil on network failure")
  assert.eq(err.status, 0, "list err status 0")
  print("  ok list network failure")
end

-- revoke success
do
  local e = fake_engine({
    ["DELETE /api/v1/engine/auth/admin/sessions/s1"] = { status = 204, body = nil },
  })
  local sdk = sessions_mod.new(e)
  local data, err = sdk.revoke("s1")
  assert.eq(err, nil, "revoke no error on 204")
  assert.eq(e.calls[1].method, "DELETE", "revoke uses DELETE")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/sessions/s1", "revoke path")
  print("  ok revoke success")
end

-- revoke 404
do
  local e = fake_engine({
    ["DELETE /api/v1/engine/auth/admin/sessions/gone"] = { status = 404, body = nil },
  })
  local sdk = sessions_mod.new(e)
  local data, err = sdk.revoke("gone")
  assert.eq(data, nil, "revoke nil on 404")
  assert.eq(err.status, 404, "revoke err 404")
  print("  ok revoke 404")
end

print("[sysops.auth.sessions] all passed")
