--! sysops.vault.me tests
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/vault/me.test.lua

local me_mod = require("sysops.vault.me")

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

print("[sysops.vault.me]")

do
  local eng = make_engine({
    ["GET /api/v1/vault/me/user-123"] = {
      status = 200,
      body = json.encode({ user_id = "user-123", cipher_count = 42 }),
    },
  })
  local me = me_mod.new(eng)
  local data, err = me.sync("user-123")
  assert.eq(err, nil, "sync: no error")
  assert.eq(eng.calls[1].method, "GET", "sync method")
  assert.eq(eng.calls[1].path, "/api/v1/vault/me/user-123", "sync path")
  assert.eq(data.user_id, "user-123", "sync user_id returned")
  print("  ok sync 200")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/me/user-123"] = { status = 404, body = "not found" },
  })
  local me = me_mod.new(eng)
  local data, err = me.sync("user-123")
  assert.eq(data, nil, "sync 404: no data")
  assert.eq(err.status, 404, "sync 404: error status")
  print("  ok sync 404")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/me/user-123"] = { status = 0, body = nil },
  })
  local me = me_mod.new(eng)
  local data, err = me.sync("user-123")
  assert.eq(data, nil, "sync network fail: no data")
  assert.eq(err.status, 0, "sync network fail: status 0")
  print("  ok sync network failure (status=0)")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/me/user-123/items"] = {
      status = 200,
      body = json.encode({ items = { { id = "cipher-1", name = "GitHub" } } }),
    },
  })
  local me = me_mod.new(eng)
  local data, err = me.ciphers("user-123")
  assert.eq(err, nil, "ciphers: no error")
  assert.eq(eng.calls[1].path, "/api/v1/vault/me/user-123/items", "ciphers path")
  assert.eq(data.items[1].name, "GitHub", "ciphers first item name")
  print("  ok ciphers 200")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/me/user-123/folders"] = {
      status = 200,
      body = json.encode({ folders = { { id = "fold-1", name = "Work" } } }),
    },
  })
  local me = me_mod.new(eng)
  local data, err = me.folders("user-123")
  assert.eq(err, nil, "folders: no error")
  assert.eq(eng.calls[1].path, "/api/v1/vault/me/user-123/folders", "folders path")
  assert.eq(data.folders[1].name, "Work", "folders first entry name")
  print("  ok folders 200")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/me/user-123/profile"] = {
      status = 200,
      body = json.encode({ email = "alice@example.com", name = "Alice" }),
    },
  })
  local me = me_mod.new(eng)
  local data, err = me.profile("user-123")
  assert.eq(err, nil, "profile: no error")
  assert.eq(eng.calls[1].path, "/api/v1/vault/me/user-123/profile", "profile path")
  assert.eq(data.email, "alice@example.com", "profile email returned")
  print("  ok profile 200")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/me/user%40example.com"] = {
      status = 200,
      body = json.encode({ user_id = "user@example.com" }),
    },
  })
  local me = me_mod.new(eng)
  local data, err = me.sync("user@example.com")
  assert.eq(err, nil, "sync encoded user_id: no error")
  assert.eq(eng.calls[1].path, "/api/v1/vault/me/user%40example.com", "sync encoded path")
  print("  ok user_id is percent-encoded")
end

print("[sysops.vault.me] all passed")
