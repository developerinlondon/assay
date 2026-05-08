--! sysops.auth.zanzibar tests
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;;' \
--!     assay libs/sysops/tests-lua/auth/zanzibar.test.lua

local zanzibar_mod = require("sysops.auth.zanzibar")

local function fake_engine(responses)
  local calls = {}
  local function do_call(method, path, body)
    table.insert(calls, { method = method, path = path, body = body })
    local key = method .. " " .. path
    -- key match may include query string; try exact first then prefix
    if responses[key] then return responses[key] end
    for k, v in pairs(responses) do
      if path:sub(1, #k) == k then return v end
    end
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

print("[sysops.auth.zanzibar]")

-- namespaces success
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/zanzibar/namespaces"] = {
      status = 200,
      body   = { { name = "doc" }, { name = "folder" } },
    },
  })
  local sdk = zanzibar_mod.new(e)
  local data, err = sdk.namespaces()
  assert.not_nil(data, "namespaces data on 200")
  assert.eq(err, nil, "namespaces no error")
  assert.eq(e.calls[1].method, "GET", "namespaces uses GET")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/zanzibar/namespaces", "namespaces path")
  print("  ok namespaces success")
end

-- namespaces 503
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/zanzibar/namespaces"] = { status = 503, body = nil },
  })
  local sdk = zanzibar_mod.new(e)
  local data, err = sdk.namespaces()
  assert.eq(data, nil, "namespaces nil on 503")
  assert.eq(err.status, 503, "namespaces err 503")
  print("  ok namespaces 503")
end

-- tuples — engine returns 404 (not yet implemented)
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/zanzibar/tuples"] = { status = 404, body = nil },
  })
  local sdk = zanzibar_mod.new(e)
  local data, err = sdk.tuples()
  assert.eq(data, nil, "tuples nil on 404")
  assert.eq(err.status, 404, "tuples err 404 — listing not shipped")
  print("  ok tuples 404 (listing not shipped)")
end

-- tuples — engine returns 405
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/zanzibar/tuples"] = { status = 405, body = nil },
  })
  local sdk = zanzibar_mod.new(e)
  local data, err = sdk.tuples()
  assert.eq(data, nil, "tuples nil on 405")
  assert.eq(err.status, 405, "tuples err 405")
  print("  ok tuples 405")
end

-- tuples network failure
do
  local e = fake_engine({
    ["GET /api/v1/engine/auth/admin/zanzibar/tuples"] = { status = 0, body = nil },
  })
  local sdk = zanzibar_mod.new(e)
  local data, err = sdk.tuples()
  assert.eq(data, nil, "tuples nil on network failure")
  assert.eq(err.status, 0, "tuples err status 0")
  print("  ok tuples network failure")
end

-- write_tuple success
do
  local e = fake_engine({
    ["POST /api/v1/engine/auth/admin/zanzibar/tuples"] = {
      status = 201,
      body   = { written = true },
    },
  })
  local sdk = zanzibar_mod.new(e)
  local t = { subject = "user:alice", relation = "viewer", object = "doc:foo" }
  local data, err = sdk.write_tuple(t)
  assert.not_nil(data, "write_tuple data on 201")
  assert.eq(err, nil, "write_tuple no error")
  assert.eq(e.calls[1].method, "POST", "write_tuple uses POST")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/zanzibar/tuples", "write_tuple path")
  assert.eq(e.calls[1].body.subject, "user:alice", "write_tuple body subject")
  print("  ok write_tuple success")
end

-- write_tuple 503
do
  local e = fake_engine({
    ["POST /api/v1/engine/auth/admin/zanzibar/tuples"] = { status = 503, body = nil },
  })
  local sdk = zanzibar_mod.new(e)
  local data, err = sdk.write_tuple({ subject = "user:x", relation = "r", object = "o:y" })
  assert.eq(data, nil, "write_tuple nil on 503")
  assert.eq(err.status, 503, "write_tuple err 503")
  print("  ok write_tuple 503")
end

-- delete_tuple success
do
  local e = fake_engine({})
  -- delete_tuple always calls DELETE; accept any path starting with the base
  local sdk = zanzibar_mod.new(e)
  local t = { subject = "user:alice", relation = "viewer", object = "doc:foo" }
  -- Patch the engine to return 204 for DELETE regardless of exact path
  e.delete = function(p)
    table.insert(e.calls, { method = "DELETE", path = p })
    return { status = 204, body = nil }
  end
  local data, err = sdk.delete_tuple(t)
  assert.eq(err, nil, "delete_tuple no error on 204")
  local last = e.calls[#e.calls]
  assert.eq(last.method, "DELETE", "delete_tuple uses DELETE")
  assert.eq(last.path:find("/api/v1/engine/auth/admin/zanzibar/tuples", 1, true) ~= nil, true, "delete_tuple base path")
  print("  ok delete_tuple success")
end

-- check success — allowed
do
  local e = fake_engine({
    ["POST /api/v1/engine/auth/admin/zanzibar/check"] = {
      status = 200,
      body   = { allowed = true },
    },
  })
  local sdk = zanzibar_mod.new(e)
  local data, err = sdk.check("user:alice", "viewer", "doc:foo")
  assert.not_nil(data, "check data on 200")
  assert.eq(err, nil, "check no error")
  assert.eq(data.allowed, true, "check allowed")
  assert.eq(e.calls[1].method, "POST", "check uses POST")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/zanzibar/check", "check path")
  assert.eq(e.calls[1].body.subject, "user:alice", "check body subject")
  assert.eq(e.calls[1].body.relation, "viewer", "check body relation")
  assert.eq(e.calls[1].body.object, "doc:foo", "check body object")
  print("  ok check success")
end

-- check 403 (denied)
do
  local e = fake_engine({
    ["POST /api/v1/engine/auth/admin/zanzibar/check"] = {
      status = 200,
      body   = { allowed = false },
    },
  })
  local sdk = zanzibar_mod.new(e)
  local data, err = sdk.check("user:bob", "owner", "doc:foo")
  assert.not_nil(data, "check data when denied")
  assert.eq(err, nil, "check no transport error when denied")
  assert.eq(data.allowed, false, "check not allowed")
  print("  ok check denied (200 body allowed=false)")
end

-- check 503
do
  local e = fake_engine({
    ["POST /api/v1/engine/auth/admin/zanzibar/check"] = { status = 503, body = nil },
  })
  local sdk = zanzibar_mod.new(e)
  local data, err = sdk.check("user:x", "r", "obj:y")
  assert.eq(data, nil, "check nil on 503")
  assert.eq(err.status, 503, "check err 503")
  print("  ok check 503")
end

-- expand success
do
  local e = fake_engine({
    ["POST /api/v1/engine/auth/admin/zanzibar/expand"] = {
      status = 200,
      body   = { tree = { type = "union", children = {} } },
    },
  })
  local sdk = zanzibar_mod.new(e)
  local data, err = sdk.expand("doc:foo", "viewer")
  assert.not_nil(data, "expand data on 200")
  assert.eq(err, nil, "expand no error")
  assert.eq(e.calls[1].method, "POST", "expand uses POST")
  assert.eq(e.calls[1].path, "/api/v1/engine/auth/admin/zanzibar/expand", "expand path")
  assert.eq(e.calls[1].body.object, "doc:foo", "expand body object")
  assert.eq(e.calls[1].body.relation, "viewer", "expand body relation")
  print("  ok expand success")
end

-- expand 404
do
  local e = fake_engine({
    ["POST /api/v1/engine/auth/admin/zanzibar/expand"] = { status = 404, body = nil },
  })
  local sdk = zanzibar_mod.new(e)
  local data, err = sdk.expand("doc:missing", "viewer")
  assert.eq(data, nil, "expand nil on 404")
  assert.eq(err.status, 404, "expand err 404")
  print("  ok expand 404")
end

print("[sysops.auth.zanzibar] all passed")
