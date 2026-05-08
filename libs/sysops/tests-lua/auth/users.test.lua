--! sysops.auth.users tests
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;;' \
--!     assay libs/sysops/tests-lua/auth/users.test.lua

local users_mod = require("sysops.auth.users")

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

print("[sysops.auth.users]")

-- list success (no filter)
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/users"] = {
      status = 200,
      body   = { items = { { id = "u1" } }, total = 1 },
    },
  })
  local sdk = users_mod.new(e)
  local data, err = sdk.list()
  assert.not_nil(data, "list data on 200")
  assert.eq(err, nil, "list no error")
  assert.eq(data.total, 1, "list total")
  assert.eq(e.calls[1].method, "GET", "list uses GET")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/users", "list path no filter")
  print("  ok list no filter")
end

-- list with search filter
do
  local e = fake_engine({})
  local sdk = users_mod.new(e)
  sdk.list({ search = "alice", limit = 10, offset = 0 })
  local path = e.calls[1].path
  assert.eq(path:find("search=alice", 1, true) ~= nil, true, "list search param")
  assert.eq(path:find("limit=10",     1, true) ~= nil, true, "list limit param")
  assert.eq(path:find("offset=0",     1, true) ~= nil, true, "list offset param")
  print("  ok list with filter")
end

-- list 404
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/users"] = { status = 404, body = nil },
  })
  local sdk = users_mod.new(e)
  local data, err = sdk.list()
  assert.eq(data, nil, "list nil on 404")
  assert.eq(err.status, 404, "list err 404")
  print("  ok list 404")
end

-- list 503
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/users"] = { status = 503, body = nil },
  })
  local sdk = users_mod.new(e)
  local data, err = sdk.list()
  assert.eq(data, nil, "list nil on 503")
  assert.eq(err.status, 503, "list err 503")
  print("  ok list 503")
end

-- list network failure
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/users"] = { status = 0, body = nil },
  })
  local sdk = users_mod.new(e)
  local data, err = sdk.list()
  assert.eq(data, nil, "list nil on network failure")
  assert.eq(err.status, 0, "list err status 0")
  print("  ok list network failure")
end

-- get by id
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/users/u1"] = {
      status = 200, body = { id = "u1", email = "alice@x.com" },
    },
  })
  local sdk = users_mod.new(e)
  local data, err = sdk.get("u1")
  assert.not_nil(data, "get data on 200")
  assert.eq(err, nil, "get no error")
  assert.eq(data.id, "u1", "get id")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/users/u1", "get path")
  print("  ok get by id")
end

-- get 404
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/users/missing"] = { status = 404, body = nil },
  })
  local sdk = users_mod.new(e)
  local data, err = sdk.get("missing")
  assert.eq(data, nil, "get nil on 404")
  assert.eq(err.status, 404, "get err 404")
  print("  ok get 404")
end

-- create success
do
  local e = fake_engine({
    ["POST /api/v1/engine/auth/admin/users"] = {
      status = 201, body = { id = "u2", email = "bob@x.com" },
    },
  })
  local sdk = users_mod.new(e)
  local data, err = sdk.create({ email = "bob@x.com", display_name = "Bob", initial_password = "pw" })
  assert.not_nil(data, "create data on 201")
  assert.eq(err, nil, "create no error")
  assert.eq(e.calls[1].method, "POST", "create uses POST")
  assert.eq(e.calls[1].body.email, "bob@x.com", "create body email")
  print("  ok create success")
end

-- update success
do
  local e = fake_engine({
    ["PUT /api/v1/engine/auth/admin/users/u1"] = {
      status = 200, body = { id = "u1", display_name = "Alice Updated" },
    },
  })
  local sdk = users_mod.new(e)
  local data, err = sdk.update("u1", { display_name = "Alice Updated" })
  assert.not_nil(data, "update data on 200")
  assert.eq(err, nil, "update no error")
  assert.eq(e.calls[1].method, "PUT", "update uses PUT")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/users/u1", "update path")
  print("  ok update success")
end

-- delete success
do
  local e = fake_engine({
    ["DELETE /api/v1/engine/auth/admin/users/u1"] = { status = 204, body = nil },
  })
  local sdk = users_mod.new(e)
  -- 204 is 2xx so result should return body (nil) with no error
  local data, err = sdk.delete("u1")
  assert.eq(err, nil, "delete no error on 204")
  assert.eq(e.calls[1].method, "DELETE", "delete uses DELETE")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/users/u1", "delete path")
  print("  ok delete success")
end

-- id URL-encoding
do
  local e = fake_engine({})
  local sdk = users_mod.new(e)
  sdk.get("user with spaces")
  assert.eq(
    e.calls[1].path,
    "/api/v1/engine/auth/admin/users/user%20with%20spaces",
    "id is URL-encoded"
  )
  print("  ok id URL-encoding")
end

print("[sysops.auth.users] all passed")
