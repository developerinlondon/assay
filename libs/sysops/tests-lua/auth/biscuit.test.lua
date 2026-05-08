--! sysops.auth.biscuit tests
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;;' \
--!     assay libs/sysops/tests-lua/auth/biscuit.test.lua

local biscuit_mod = require("sysops.auth.biscuit")

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

print("[sysops.auth.biscuit]")

-- info success
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/biscuit"] = {
      status = 200,
      body   = { kid = "bk1", public_pem = "-----BEGIN PUBLIC KEY-----\n..." },
    },
  })
  local sdk = biscuit_mod.new(e)
  local data, err = sdk.info()
  assert.not_nil(data, "info data on 200")
  assert.eq(err, nil, "info no error")
  assert.eq(data.kid, "bk1", "info kid")
  assert.eq(e.calls[1].method, "GET", "info uses GET")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/biscuit", "info path")
  print("  ok info success")
end

-- info 404
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/biscuit"] = { status = 404, body = nil },
  })
  local sdk = biscuit_mod.new(e)
  local data, err = sdk.info()
  assert.eq(data, nil, "info nil on 404")
  assert.eq(err.status, 404, "info err 404")
  print("  ok info 404")
end

-- info 503
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/biscuit"] = { status = 503, body = nil },
  })
  local sdk = biscuit_mod.new(e)
  local data, err = sdk.info()
  assert.eq(data, nil, "info nil on 503")
  assert.eq(err.status, 503, "info err 503")
  print("  ok info 503")
end

-- info network failure
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/biscuit"] = { status = 0, body = nil },
  })
  local sdk = biscuit_mod.new(e)
  local data, err = sdk.info()
  assert.eq(data, nil, "info nil on network failure")
  assert.eq(err.status, 0, "info err status 0")
  print("  ok info network failure")
end

print("[sysops.auth.biscuit] all passed")
