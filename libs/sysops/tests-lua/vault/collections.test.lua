--! sysops.vault.collections tests
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/vault/collections.test.lua

local collections_mod = require("sysops.vault.collections")

local function make_engine(responses)
  local calls = {}
  local eng = {}
  eng.calls = calls

  local function do_call(method, path, body)
    table.insert(calls, { method = method, path = path, body = body })
    local key = method .. " " .. path
    if responses[key] then return responses[key] end
    return { status = 200, body = json.encode({}) }
  end

  function eng.get(path)        return do_call("GET",    path) end
  function eng.post(path, body) return do_call("POST",   path, body) end
  function eng.put(path, body)  return do_call("PUT",    path, body) end
  function eng.delete(path)     return do_call("DELETE", path) end

  return eng
end

print("[sysops.vault.collections]")

do
  local eng = make_engine({
    ["GET /api/v1/vault/folders"] = {
      status = 200,
      body = json.encode({ collections = { { id = "col-1", name = "Shared" } } }),
    },
  })
  local col = collections_mod.new(eng)
  local data, err = col.list()
  assert.eq(err, nil, "list: no error")
  assert.eq(eng.calls[1].method, "GET", "list method")
  assert.eq(eng.calls[1].path, "/api/v1/vault/folders", "list path")
  assert.eq(data.collections[1].name, "Shared", "list first collection name")
  print("  ok list 200")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/folders"] = { status = 503, body = "module disabled" },
  })
  local col = collections_mod.new(eng)
  local data, err = col.list()
  assert.eq(data, nil, "list 503: no data")
  assert.eq(err.status, 503, "list 503: error status")
  print("  ok list 503")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/folders"] = { status = 0, body = nil },
  })
  local col = collections_mod.new(eng)
  local data, err = col.list()
  assert.eq(data, nil, "list network fail: no data")
  assert.eq(err.status, 0, "list network fail: status 0")
  print("  ok list network failure (status=0)")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/folders"] = {
      status = 201,
      body = json.encode({ id = "col-2", name = "Engineering", description = "Eng team secrets" }),
    },
  })
  local col = collections_mod.new(eng)
  local data, err = col.create("Engineering", "Eng team secrets")
  assert.eq(err, nil, "create: no error")
  assert.eq(eng.calls[1].method, "POST", "create method")
  assert.eq(eng.calls[1].path, "/api/v1/vault/folders", "create path")
  assert.eq(eng.calls[1].body.name, "Engineering", "create body.name")
  assert.eq(eng.calls[1].body.description, "Eng team secrets", "create body.description")
  assert.eq(data.id, "col-2", "create id returned")
  print("  ok create 201")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/folders"] = { status = 409, body = "already exists" },
  })
  local col = collections_mod.new(eng)
  local data, err = col.create("Engineering", "")
  assert.eq(data, nil, "create 409: no data")
  assert.eq(err.status, 409, "create 409: error status")
  print("  ok create 409 conflict")
end

print("[sysops.vault.collections] all passed")
