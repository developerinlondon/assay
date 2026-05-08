--! sysops.auth.audit tests
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;;' \
--!     assay libs/sysops/tests-lua/auth/audit.test.lua

local audit_mod = require("sysops.auth.audit")

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

print("[sysops.auth.audit]")

-- list success (no filter)
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/audit"] = {
      status = 200,
      body   = { items = { { id = "ev1", kind = "login" } }, total = 1, enabled = true },
    },
  })
  local sdk = audit_mod.new(e)
  local data, err = sdk.list()
  assert.not_nil(data, "list data on 200")
  assert.eq(err, nil, "list no error")
  assert.eq(data.total, 1, "list total")
  assert.eq(e.calls[1].method, "GET", "list uses GET")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/audit", "list path no filter")
  print("  ok list no filter")
end

-- list with filters
do
  local e = fake_engine({})
  local sdk = audit_mod.new(e)
  sdk.list({ limit = 20, offset = 40, since = "2026-01-01", kind = "login" })
  local path = e.calls[1].path
  assert.eq(path:find("limit=20",          1, true) ~= nil, true, "list limit param")
  assert.eq(path:find("offset=40",         1, true) ~= nil, true, "list offset param")
  assert.eq(path:find("kind=login",        1, true) ~= nil, true, "list kind param")
  assert.eq(path:find("since=2026", 1, true) ~= nil, true, "list since param")
  print("  ok list with filters")
end

-- list 404
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/audit"] = { status = 404, body = nil },
  })
  local sdk = audit_mod.new(e)
  local data, err = sdk.list()
  assert.eq(data, nil, "list nil on 404")
  assert.eq(err.status, 404, "list err 404")
  print("  ok list 404")
end

-- list 503 (audit module disabled at engine compile time)
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/audit"] = { status = 503, body = nil },
  })
  local sdk = audit_mod.new(e)
  local data, err = sdk.list()
  assert.eq(data, nil, "list nil on 503")
  assert.eq(err.status, 503, "list err 503")
  print("  ok list 503")
end

-- list network failure
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/audit"] = { status = 0, body = nil },
  })
  local sdk = audit_mod.new(e)
  local data, err = sdk.list()
  assert.eq(data, nil, "list nil on network failure")
  assert.eq(err.status, 0, "list err status 0")
  print("  ok list network failure")
end

print("[sysops.auth.audit] all passed")
